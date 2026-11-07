# Classical printf stepping fixture

`launch.bennett` is the LLVM 18 mixed-vararg musl printf object produced by
plugin commit 211f413. SHA256:
`5d1edcef6a5ae241178506cca117c7f6564629e477df29a3f262727770dfb302`.
`target.json` is the published ideal-all-to-all target at targets commit
`dd036711936c496f53f6f1a9bebeb686335bf7cc`, which the producer's C ABI test installs.

`reference.json` records the independent reference interpreter run: status 0,
225956 steps and exactly 54 console bytes (`iplqudez\n` six times).
The oracle test reruns the reference binary at the VM's gate pin and checks
these values, then checks the VM's status, rows, console and target time.
Wall times are informational; the hang guard reports UNKNOWN if it expires.
`replies.json` supplies the single 20-byte empty argv response; its payload is
the object's zeroed argv buffer.
