//! K82 wire interfaces for the device constructor language.
use super::*;

pub(super) fn collect(doc: &Json) -> Result<BTreeMap<String, Vec<String>>, String> {
    struct Analyzer<'a> {
        recipes: BTreeMap<&'a str, &'a Json>,
        cache: BTreeMap<String, Vec<String>>,
        active: BTreeSet<String>,
    }
    impl Analyzer<'_> {
        fn recipe(&mut self, name: &str) -> Result<Option<Vec<String>>, String> {
            if let Some(interface) = self.cache.get(name) {
                return Ok(Some(interface.clone()));
            }
            let recipe = *self
                .recipes
                .get(name)
                .ok_or_else(|| err("unknown invoked recipe"))?;
            let formals = arr(field(recipe, "parameters")?)?;
            if !formals.iter().all(|p| p.as_str().is_some()) {
                return Ok(None);
            }
            if !self.active.insert(name.into()) {
                return Err(err("cyclic named interface"));
            }
            let bound = formals
                .iter()
                .map(|p| string(p).map(String::from))
                .collect::<Result<BTreeSet<_>, _>>()?;
            let mut interface = vec![];
            self.walk(field(recipe, "body")?, &bound, &mut interface)?;
            self.active.remove(name);
            self.cache.insert(name.into(), interface.clone());
            Ok(Some(interface))
        }
        fn name(name: &str, bound: &BTreeSet<String>, out: &mut Vec<String>) {
            if !bound.contains(name) && !out.iter().any(|n| n == name) {
                out.push(name.into());
            }
        }
        fn wire(
            value: &Json,
            bound: &BTreeSet<String>,
            out: &mut Vec<String>,
        ) -> Result<(), String> {
            match value {
                Json::Str(name) => Self::name(name.split('[').next().unwrap(), bound, out),
                Json::Arr(values) => {
                    for value in values {
                        Self::wire(value, bound, out)?;
                    }
                }
                _ => Self::wire(field(value, "range")?, bound, out)?,
            }
            Ok(())
        }
        fn walk(
            &mut self,
            rows: &Json,
            inherited: &BTreeSet<String>,
            out: &mut Vec<String>,
        ) -> Result<(), String> {
            let mut bound = inherited.clone();
            for row in arr(rows)? {
                if row.get("gate").is_some() || row.get("image").is_some() {
                    Self::wire(field(row, "wires")?, &bound, out)?;
                } else if let Some(name) = row.get("alloc") {
                    bound.insert(string(name)?.into());
                } else if let Some(name) = row.get("declare") {
                    let mut local = bound.clone();
                    local.insert(string(name)?.into());
                    self.walk(field(row, "body")?, &local, out)?;
                } else if let Some(name) = row
                    .get("release")
                    .or_else(|| row.get("acquire"))
                    .or_else(|| row.get("relinquish"))
                {
                    Self::name(string(name)?, &bound, out);
                } else if let Some(name) = row.get("each").or_else(|| row.get("for")) {
                    if row.get("each").is_some() {
                        Self::wire(field(row, "in")?, &bound, out)?;
                    }
                    let mut local = bound.clone();
                    local.insert(string(name)?.into());
                    self.walk(field(row, "body")?, &local, out)?;
                } else if let Some(kind) = row.get("invoke") {
                    let kind = string(kind)?;
                    let recipe = *self
                        .recipes
                        .get(kind)
                        .ok_or_else(|| err("unknown invoked recipe"))?;
                    let formals = arr(field(recipe, "parameters")?)?;
                    let args = arr(field(row, "args")?)?;
                    if formals.len() != args.len() {
                        return Err(err("invoke arity mismatch"));
                    }
                    for (formal, arg) in formals.iter().zip(args) {
                        match formal.get("type").and_then(Json::as_str) {
                            Some("wire" | "environment") => Self::wire(arg, &bound, out)?,
                            Some("body") if arg.as_arr().is_some() => {
                                self.walk(arg, &bound, out)?
                            }
                            _ => {}
                        }
                    }
                    if let Some(interface) = self.recipe(kind)? {
                        for name in interface {
                            Self::name(&name, &bound, out);
                        }
                    }
                } else if let Some(controls) = row.get("under") {
                    Self::wire(controls, &bound, out)?;
                    self.walk(field(row, "body")?, &bound, out)?;
                } else if let Some(body) = row.get("body") {
                    self.walk(body, &bound, out)?;
                }
            }
            Ok(())
        }
    }
    let recipes = arr(field(doc, "recipes")?)?
        .iter()
        .map(|r| Ok((string(field(r, "kind")?)?, r)))
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let names = recipes.keys().copied().collect::<Vec<_>>();
    let mut analyzer = Analyzer {
        recipes,
        cache: BTreeMap::new(),
        active: BTreeSet::new(),
    };
    for name in names {
        analyzer.recipe(name)?;
    }
    Ok(analyzer.cache)
}

pub(super) fn scope(outer: &Bindings, names: Option<&[String]>) -> Result<Bindings, String> {
    // Keep invocation ownership, controls, registry and captured execution
    // context, while exposing only this recipe's explicit lexical inputs.
    let mut scope = outer.clone();
    scope.values.clear();
    scope.locals.clear();
    if let Some(names) = names {
        for name in names {
            let actual = wires(&Json::Str(name.clone()), outer)?;
            let value = match outer.values.get(name) {
                Some(Bound::Allocation(a)) => Bound::Allocation(a.clone()),
                _ => Bound::Wire(actual),
            };
            scope.values.insert(name.clone(), value);
        }
    }
    Ok(scope)
}
