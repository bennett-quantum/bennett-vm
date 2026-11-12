use super::*;

#[test]
fn pure_history_row_reserves_less_than_a_data_snapshot() {
    let frame = std::mem::size_of::<Frame>();
    let data = std::mem::size_of::<Data>();
    eprintln!("retained frame bytes={frame}, data snapshot bytes={data}");
    assert!(
        frame < data,
        "pure rows reserve inline optional state snapshots"
    );
}
