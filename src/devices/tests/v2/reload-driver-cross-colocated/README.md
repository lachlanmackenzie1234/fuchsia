# Reload Driver Cross-Colocated Test

This test checks that DFv2 driver reload/restart works properly when drivers are colocated across independent branches of the node DAG using a custom string-based driver host tag (`driver_host = "shared-host"`).

## Scenario

This is the node topology that will be tested. The conventions for the graph below are:
 - `X` in the edges indicates `colocate=false` in the child node addition. Otherwise it is `true`.
 - Node markings are in the form `NodeName(Host ID)`.

```
               dev(root)
              /         \
             X           X
            /             \
      left_parent     right_parent
       (Host 1)         (Host 2)
          |                 |
          X                 X
          |                 |
       child_a           child_b
     (driver=target)    (driver=leaf)
       (Host 3)           (Host 3)
```

## Details

1. `dev` (root driver) creates `left_parent` and `right_parent` as child nodes with `colocate = false`. They are placed into separate driver host instances (`Host 1` and `Host 2`).
2. `left_parent` driver adds child node `child_a` with `driver_host = "shared-host"`.
3. `right_parent` driver adds child node `child_b` with `driver_host = "shared-host"`.
4. `target` driver (`target.cm`) binds to `child_a`.
5. `leaf` driver (`leaf.cm`) binds to `child_b`.
6. Since both `child_a` and `child_b` specify `driver_host = "shared-host"`, they share the same driver host process (`Host 3`, moniker `driver-host-shared-host`).

## Expectation

When `target` driver is restarted via `ffx driver restart fuchsia-boot:///#meta/target.cm`:
 - Even though only `target` (on `child_a`) was restarted, `child_b` (running `leaf`) is **also restarted** into a **new** driver host process because it resides in the same colocated driver host.
 - The host process KOIDs for `child_a` and `child_b` after restart should be **identical to each other** and **different from their initial host process KOID**.
 - `left_parent` and `right_parent` must **not** be restarted and must retain their initial host process KOIDs.
