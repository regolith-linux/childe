use clap::Parser;
use human_panic::setup_panic;
use std::error::Error;
use swayipc::{Connection, Fallible};
use trawlcat::rescat;

/// Simple command line utility to find and move to the next unallocated workspace.
#[derive(Parser, Debug)]
#[command(name = "childe", version, about)]
struct CliArgs {
    #[arg(short, long = "move-window")]
    move_window: bool,
    #[arg(short, long = "follow", requires = "move_window")]
    follow: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Init
    setup_panic!();
    let args = CliArgs::parse();
    let mut sway_cn = Connection::new().expect("Cannot create Sway IPC connection");

    // Find unallocated workspace
    let workspace_name = get_workspace_name(&mut sway_cn)
        .await
        .expect("Cannot get workspace name");

    // Move and navigate based on params
    if args.move_window {
        sway_cn
            .run_command(format!("move window to workspace {workspace_name}"))
            .expect("Cannot run command");
    }
    if !args.move_window || args.follow {
        sway_cn
            .run_command(format!("workspace {workspace_name}"))
            .expect("Cannot run command");
    }

    Ok(())
}

// return index of active workspaces
fn workspace_nums(connection: &mut Connection) -> Fallible<Vec<i32>> {
    let mut workspace_nums:Vec<_> = connection
        .get_workspaces()?
        .iter()
        .map(|ws| ws.num)
        .collect();

    workspace_nums.sort();

    Ok(workspace_nums)
}

// assumes input list is sorted in ascending order
fn find_gap(items: &[i32]) -> i32 {
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
