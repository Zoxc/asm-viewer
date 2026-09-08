use super::*;

#[test]
fn a_list_is_equal_to_its_own_clone_and_to_no_other_build() {
    let rows: Shared<u32> = vec![1, 2, 3].into();
    assert_eq!(rows, rows.clone());
    assert_ne!(rows, Shared::from(vec![1, 2, 3]));
}

#[test]
fn an_optional_arc_is_the_same_one_only_when_both_are_that_build() {
    let one = Arc::new(1);
    let other = Arc::new(1);

    assert!(same_arc::<u32>(&None, &None));
    assert!(same_arc(&Some(one.clone()), &Some(one.clone())));
    assert!(!same_arc(&Some(one.clone()), &Some(other)));
    assert!(!same_arc(&Some(one.clone()), &None));
    assert!(!same_arc(&None, &Some(one)));
}
