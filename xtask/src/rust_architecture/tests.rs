use super::{Baseline, Exception, ModuleSize, violations};
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Uses one concrete owner so policy tests compare the full diagnostic contract.
fn module(lines: usize) -> BTreeMap<PathBuf, ModuleSize> {
    BTreeMap::from([(
        PathBuf::from("module.rs"),
        ModuleSize {
            owner: "ora-example".into(),
            production_lines: lines,
            test_only: false,
        },
    )])
}

/// Records a real extraction direction rather than an anonymous allowance.
fn baseline(lines: usize) -> Baseline {
    Baseline { modules: BTreeMap::from([(PathBuf::from("module.rs"), Exception {
        owner: "ora-example".into(), lines,
        split_plan: "Extract lifecycle recovery and its storage-backed tests into the recovery module.".into(),
    })]) }
}

/// The 500-line target is advisory, while unreviewed production growth beyond 800 is blocked.
#[test]
fn enforces_the_hard_limit_without_mislabeling_the_target() {
    let empty = Baseline {
        modules: BTreeMap::new(),
    };
    assert_eq!(violations(&module(800), &empty), Vec::<String>::new());
    assert_eq!(
        violations(&module(801), &empty),
        vec!["module.rs: new oversized production module (801 > 800); split by responsibility"]
    );
}

/// A reviewed exception can remain unchanged, but cannot grow or keep obsolete headroom.
#[test]
fn ratchets_existing_debt_in_both_directions() {
    let baseline = baseline(900);
    assert_eq!(violations(&module(900), &baseline), Vec::<String>::new());
    assert_eq!(
        violations(&module(901), &baseline),
        vec![
            "module.rs: production module grew from 900 to 901 lines; extract the new responsibility"
        ]
    );
    assert_eq!(
        violations(&module(899), &baseline),
        vec!["module.rs: lower the baseline from 900 to 899 so removed debt cannot grow back"]
    );
}

/// Deleting or successfully splitting a module also removes its exception.
#[test]
fn rejects_stale_exceptions() {
    let baseline = baseline(900);
    assert_eq!(
        violations(&BTreeMap::new(), &baseline),
        vec!["module.rs: remove the exception for a deleted module"]
    );
    assert_eq!(
        violations(&module(800), &baseline),
        vec!["module.rs: remove the exception now that the module is within 800 lines"]
    );
}

/// Every exceptional module remains attached to its current Cargo owner and a concrete plan.
#[test]
fn rejects_unowned_or_unplanned_debt() {
    let mut baseline = baseline(900);
    let exception = baseline
        .modules
        .get_mut(&PathBuf::from("module.rs"))
        .unwrap();
    exception.owner = "someone".into();
    exception.split_plan = "later".into();
    assert_eq!(
        violations(&module(900), &baseline),
        vec!["module.rs: exception needs the owning crate (ora-example) and a concrete split plan"]
    );
}
