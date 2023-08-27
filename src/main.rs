use std::{error::Error, process::Command};
use swayipc::{Connection, Fallible};

fn main() -> Result<(), Box<dyn Error>> {
    let mut sway_cn = Connection::new()?;
    let gap_index = find_gap(&workspace_nums(&mut sway_cn)?);
    let resource_name = format!("wm.workspace.{:02}.name", gap_index);

    let workspace_name_raw = Command::new("trawlcat")
        .args([resource_name, format!("number {gap_index}")])
        .output()
        .unwrap()
        .stdout;
    let workspace_name = String::from_utf8(workspace_name_raw).unwrap();
    sway_cn.run_command(format!("workspace {workspace_name}"))?;
    Ok(())
}

// return index of active workspaces
fn workspace_nums(connection: &mut Connection) -> Fallible<Vec<i32>> {
    Ok(connection.get_workspaces()?.iter().map(|ws| ws.num).collect())
}

// assumes input list is already sorted in ascending order
fn find_gap(items: &Vec<i32>) -> i32 {
    match items.len() {
        0 => return 1, // empty list should take index 1
        _ => {
            let mut iter = items.iter().peekable();

            while let Some(cv) = iter.next() {
                match iter.peek() {
                    Some(nv) => {
                        if **nv != cv + 1 { return cv + 1 } // next element greater than current + 1
                    }
                    _ => return cv + 1 // last element
                }
            }
        
            return items.last().expect("not empty") + 1 // fallback to return last element + 1
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_find_gap_empty() {
        let td = vec![];

        let expected = 1;
        let actual = find_gap(&td);

        assert_eq!(expected, actual);
    }

    #[test]
    fn test_find_gap_single() {
        let td = vec![1];

        let expected = 2;
        let actual = find_gap(&td);

        assert_eq!(expected, actual);
    }

    #[test]
    fn test_find_gap_double() {
        let td = vec![1, 2];

        let expected = 3;
        let actual = find_gap(&td);

        assert_eq!(expected, actual);
    }

    #[test]
    fn test_find_gap_base() {
        let td = vec![1, 3];

        let expected = 2;
        let actual = find_gap(&td);

        assert_eq!(expected, actual);
    }

}