using Workerd = import "/workerd/workerd.capnp";
const config :Workerd.Config = (
  services = [
    (name = "acceptance-after", worker = (
      modules = [
        (name = "acceptance.test.mjs", esModule = embed "acceptance.test.mjs"),
        (name = "worker-under-test.mjs", esModule = embed "worker-under-test.mjs")
      ],
      compatibilityDate = "2026-11-04",
      globalOutbound = "fixture",
      bindings = [(name = "FIXTURE", service = "fixture")]
    )),
    (name = "fixture", worker = (
      modules = [(name = "outbound-fixture.mjs", esModule = embed "outbound-fixture.mjs")],
      compatibilityDate = "2026-11-04"
    ))
  ]
);
