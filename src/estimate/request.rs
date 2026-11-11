//! Owned launch request identity; parameters never include outer budget or aliases.
use crate::{
    identity::Identities,
    invocation,
    json::Json,
    object::{FnKind, Object},
    sched,
    target::{Rational, Target},
    value::{Budget, Value},
};

#[derive(Clone, Debug)]
pub struct ResolvedRequest {
    region: String,
    variant: String,
    inherited_controls: u32,
    invocation: Json,
    params_hash: String,
    args: Vec<Value>,
    budget: Budget,
}
impl ResolvedRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        object: &Object,
        region: &str,
        variant: &str,
        inherited_controls: u32,
        budget: Budget,
        params: Json,
        aliases: Option<Json>,
    ) -> Result<Self, String> {
        if object.isa != "0.9" {
            return Err("§10.1: launch 0.9 estimate requires ISA 0.9".into());
        }
        if variant.is_empty() {
            return Err("§10.1: empty estimate variant".into());
        }
        let region = region.trim_start_matches('@');
        let params_hash = invocation::params_hash(&params);
        let mut invocation = Json::obj(vec![("args", params), ("budget", budget_json(budget))]);
        if let (Some(aliases), Json::Obj(fields)) = (aliases, &mut invocation) {
            fields.insert("token_aliases".into(), aliases);
        }
        let (args, budget) = invocation::resolve(object, region, &invocation)?;
        if invocation.get("token_aliases").is_none() {
            let oracle = object
                .function(region)
                .is_some_and(|f| f.kind == FnKind::Oracle);
            let aliases = Json::Arr(if oracle { vec![Json::u64(0)] } else { vec![] });
            if let Json::Obj(fields) = &mut invocation {
                fields.insert("token_aliases".into(), aliases);
            }
        }
        Ok(Self {
            region: region.into(),
            variant: variant.into(),
            inherited_controls,
            invocation,
            params_hash,
            args,
            budget,
        })
    }
    pub fn region(&self) -> &str {
        &self.region
    }
    pub fn variant(&self) -> &str {
        &self.variant
    }
    pub fn inherited_controls(&self) -> u32 {
        self.inherited_controls
    }
    pub fn invocation(&self) -> &Json {
        &self.invocation
    }
    pub fn params_hash(&self) -> &str {
        &self.params_hash
    }
    pub fn args(&self) -> &[Value] {
        &self.args
    }
    pub fn budget(&self) -> Budget {
        self.budget
    }
    pub fn params(&self) -> &Json {
        self.invocation.get("args").expect("resolved args")
    }
    pub fn token_aliases(&self) -> &Json {
        self.invocation
            .get("token_aliases")
            .expect("resolved partition")
    }
}
fn budget_json(budget: Budget) -> Json {
    Json::Arr(
        [budget.steps, budget.heap, budget.stack, budget.depth]
            .iter()
            .map(|n| Json::Str(n.to_string()))
            .collect(),
    )
}

/// The original bytes are owned; the requested value is an exact rational.
#[derive(Clone, Debug)]
pub struct ExplicitAccuracy {
    source: String,
    value: Rational,
}
impl ExplicitAccuracy {
    pub fn parse(source: &str) -> Result<Self, String> {
        if source.is_empty() || !source.is_ascii() {
            return Err("§10.1: epsilon_decimal must be an ASCII decimal".into());
        }
        let value = source.parse::<Rational>()?;
        if value.negative() {
            return Err("§10.1: epsilon_decimal must be nonnegative and finite".into());
        }
        Ok(Self {
            source: source.into(),
            value,
        })
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn value(&self) -> &Rational {
        &self.value
    }
    pub fn decimal(&self) -> String {
        self.value.decimal()
    }
    pub fn default_share(
        &self,
        inexact_operations: usize,
        exact_native: bool,
    ) -> Result<Rational, String> {
        if exact_native || inexact_operations == 0 {
            return Ok(Rational::zero());
        }
        self.value.div(&Rational::from(inexact_operations as u64))
    }
}

#[derive(Clone, Debug)]
pub struct AccuracyRequest {
    request: ResolvedRequest,
    accuracy: ExplicitAccuracy,
}
impl AccuracyRequest {
    pub fn new(request: ResolvedRequest, accuracy: ExplicitAccuracy) -> Self {
        Self { request, accuracy }
    }
    pub fn request(&self) -> &ResolvedRequest {
        &self.request
    }
    pub fn accuracy(&self) -> &ExplicitAccuracy {
        &self.accuracy
    }
    pub fn effective_schedule(
        &self,
        object: &Object,
        ids: &Identities,
        target: &Target,
        attached: Option<&Json>,
    ) -> Result<EffectiveSchedule, String> {
        let expected = self.default_schedule(ids, target)?;
        if let Some(attached) = attached {
            if self.matches(attached, &expected, object)? {
                let b = self.bindings();
                sched::consume(attached, object, ids, target, &b)?;
                return Ok(EffectiveSchedule::new(attached.clone(), true));
            }
        }
        sched::consume(&expected, object, ids, target, &self.bindings())?;
        Ok(EffectiveSchedule::new(expected, false))
    }
    fn bindings(&self) -> sched::Bindings<'_> {
        let b = self.request.budget;
        sched::Bindings {
            region: self.request.region(),
            variant: self.request.variant(),
            inherited_controls: self.request.inherited_controls(),
            budget: [b.steps, b.heap, b.stack, b.depth],
            params: self.request.params(),
            token_aliases: Some(self.request.token_aliases()),
            epsilon: self.accuracy.value(),
        }
    }
    fn matches(&self, document: &Json, expected: &Json, object: &Object) -> Result<bool, String> {
        let binding = crate::target::field(document, "bindings")?;
        let expected = crate::target::field(expected, "bindings")?;
        for key in [
            "closure_hash",
            "target_hash",
            "region",
            "variant",
            "inherited_controls",
            "params_hash",
        ] {
            if binding.get(key) != expected.get(key) {
                return Ok(false);
            }
        }
        let oracle = object
            .function(self.request.region())
            .is_some_and(|f| f.kind == FnKind::Oracle);
        let inferred = Json::Arr(if oracle { vec![Json::u64(0)] } else { vec![] });
        if binding.get("token_aliases").unwrap_or(&inferred) != self.request.token_aliases() {
            return Ok(false);
        }
        let actual = crate::target::arr(crate::target::field(binding, "budget")?)?;
        let budget = self.request.budget;
        if actual.len() != 4
            || actual
                .iter()
                .zip([budget.steps, budget.heap, budget.stack, budget.depth])
                .any(|(a, b)| a.as_u64() != Some(b))
        {
            return Ok(false);
        }
        Ok(
            crate::target::decimal(crate::target::field(binding, "epsilon")?)?
                == *self.accuracy.value(),
        )
    }
    fn default_schedule(&self, ids: &Identities, target: &Target) -> Result<Json, String> {
        let closure = ids
            .closure
            .get(self.request.region())
            .and_then(Option::as_ref)
            .ok_or("§10.1: estimate region closure unavailable")?;
        Ok(Json::obj(vec![
            (
                "bindings",
                Json::obj(vec![
                    ("closure_hash", Json::Str(closure.clone())),
                    ("target_hash", Json::Str(target.hash.clone())),
                    ("region", Json::Str(self.request.region.clone())),
                    ("variant", Json::Str(self.request.variant.clone())),
                    (
                        "inherited_controls",
                        Json::u64(self.request.inherited_controls as u64),
                    ),
                    ("budget", budget_json(self.request.budget)),
                    ("params_hash", Json::Str(self.request.params_hash.clone())),
                    ("token_aliases", self.request.token_aliases().clone()),
                    ("epsilon", Json::Str(self.accuracy.decimal())),
                    (
                        "expansion",
                        Json::obj(vec![
                            ("name", Json::Str("ref".into())),
                            ("version", Json::Str("0.9".into())),
                            ("hash", Json::Str(crate::ref_library::HASH.into())),
                        ]),
                    ),
                    (
                        "scheduler",
                        Json::obj(vec![
                            ("name", Json::Str("bennett-vm".into())),
                            ("version", Json::Str(env!("CARGO_PKG_VERSION").into())),
                        ]),
                    ),
                ]),
            ),
            ("entries", Json::Arr(vec![])),
        ]))
    }
}
#[derive(Clone, Debug)]
pub struct EffectiveSchedule {
    document: Json,
    hash: String,
    attached: bool,
}
impl EffectiveSchedule {
    fn new(document: Json, attached: bool) -> Self {
        let hash = crate::sha256::sha256_hex(document.to_jcs().as_bytes());
        Self {
            document,
            hash,
            attached,
        }
    }
    pub fn document(&self) -> &Json {
        &self.document
    }
    pub fn hash(&self) -> &str {
        &self.hash
    }
    pub fn uses_attached(&self) -> bool {
        self.attached
    }
}
