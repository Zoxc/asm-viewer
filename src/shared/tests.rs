use super::*;

#[test]
fn a_list_is_equal_to_its_own_clone_and_to_no_other_build() {
    let rows: Shared<u32> = vec![1, 2, 3].into();
    assert_eq!(rows, rows.clone());
    assert_ne!(rows, Shared::from(vec![1, 2, 3]));
}
