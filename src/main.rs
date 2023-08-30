use std::{error::Error, process::Command};
use swayipc::{Connection, Fallible};
use trawlcat::rescat;
use clap::Parser;

/// Simple command line utility to find the next non-opened workspace and go there.
#[derive(Parser, Debug)]
#[command(name="childe", version, about)]
struct CliArgs {
    #[arg(short, long="move-window")]
    move_window: bool,
    #[arg(short, long="follow", requires="move_window")]
    follow: bool
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut sway_cn = Connection::new()?;
    let workspace_name = get_workspace_name(&mut sway_cn).await?;
    Ok(())
}

// return index of active workspaces
fn workspace_nums(connection: &mut Connection) -> Fallible<Vec<i32>> {
    Ok(connection
        .get_workspaces()?
        .iter()
        .map(|ws| ws.num)
        .collect())
}

// assumes input list is already sorted in ascending order
fn find_gap(items: &Vec<i32>) -> i32 {
    // Get the enumerated iterator of form (position, workspace_number)
    let iter = items.iter().enumerate();
    for (pos, &wn) in iter {
        let expected = pos as i32 + 1;
        if wn != expected {
            return expected;
        }
    }

    // Return last element or 1
    items.last().unwrap_or(&0) + 1
}

// return the name of the next empty workspace
async fn get_workspace_name(conn: &mut Connection) -> Result<String, Box<dyn Error>> {
    let gap_index = find_gap(&workspace_nums(conn)?);
    let resource_name = format!("wm.workspace.{:02}.name", gap_index);
    let workspace_name = rescat(&resource_name, Some(format!("number {gap_index}"))).await?;
    Ok(workspace_name)
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

    #[test]
    fn test_find_gap_base_2() {
        let td = vec![2, 3];

        let expected = 1;
        let actual = find_gap(&td);

        assert_eq!(expected, actual);
    }
}
