mod ipc;
mod names;
mod resources;

use clap::Parser;
use human_panic::setup_panic;
use ipc::Connection;
use resources::Resources;
use serde_json::Value;
use std::error::Error;

/// Simple command line utility to find and move to the next unallocated workspace.
#[derive(Parser, Debug)]
#[command(name = "childe", version, about)]
struct CliArgs {
    #[arg(short, long = "move-window")]
    move_window: bool,
    #[arg(short, long = "follow", requires = "move_window")]
    follow: bool,
    /// Keep running, and keep every workspace's name showing the apps on it
    #[arg(short = 'w', long = "workspace-names", conflicts_with = "move_window")]
    workspace_names: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    // Init
    setup_panic!();
    let args = CliArgs::parse();

    if args.workspace_names {
        std::process::exit(names::run());
    }

    let mut conn = Connection::connect().expect("Cannot create window manager IPC connection");

    // Find unallocated workspace
    let target = get_workspace_target(&mut conn).expect("Cannot get workspace name");

    // Move and navigate based on params
    conn.run_command(&navigation_command(&target, args.move_window, args.follow))
        .expect("Cannot run command");

    Ok(())
}

// the move and the switch, as one command list. The window manager runs a
// whole list before it reads the next request from any client, so a
// `childe --workspace-names` daemon cannot rename the new workspace between
// the move and the switch, and the switch still finds it under `target`.
fn navigation_command(target: &str, move_window: bool, follow: bool) -> String {
    let mut commands = Vec::new();
    if move_window {
        commands.push(format!("move window to workspace {target}"));
    }
    if !move_window || follow {
        commands.push(format!("workspace {target}"));
    }
    commands.join("; ")
}

// return index of active workspaces
fn workspace_nums(connection: &mut Connection) -> std::io::Result<Vec<i32>> {
    let workspaces = connection.get_workspaces()?;
    let nums = workspaces
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|ws| ws.get("num").and_then(Value::as_i64))
                .map(|num| num as i32)
                .collect()
        })
        .unwrap_or_default();

    Ok(numbered_workspaces(nums))
}

// keep only the workspace numbers that occupy a slot, sorted and without
// duplicates. sway reports -1 for named workspaces without a number (such as
// "music"), and several workspaces can share a number ("2" and "2:web"); left
// in, either breaks find_gap.
fn numbered_workspaces(mut nums: Vec<i32>) -> Vec<i32> {
    nums.retain(|&num| num > 0);
    nums.sort_unstable();
    nums.dedup();
    nums
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

// return the workspace argument of the next empty workspace: its name from
// the session's resources, or its number
fn get_workspace_target(conn: &mut Connection) -> Result<String, Box<dyn Error>> {
    let gap_index = find_gap(&workspace_nums(conn)?);
    let resource_name = format!("wm.workspace.{:02}.name", gap_index);
    let mut resources = Resources::for_session(conn.is_sway()?);
    Ok(match resources.get(&resource_name) {
        Some(name) => conn.quote(&name)?,
        None => format!("number {gap_index}"),
    })
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

    #[test]
    fn test_named_workspace_without_number() {
        // workspace 1, workspace 2 and a workspace named "music" (num -1)
        let td = numbered_workspaces(vec![1, 2, -1]);

        assert_eq!(vec![1, 2], td);
        assert_eq!(3, find_gap(&td));
    }

    #[test]
    fn test_only_named_workspaces() {
        let td = numbered_workspaces(vec![-1, -1]);

        assert_eq!(1, find_gap(&td));
    }

    #[test]
    fn test_duplicate_workspace_numbers() {
        // "1", "2" and "2:web" all exist; 3 is the first free number
        let td = numbered_workspaces(vec![2, 1, 2]);

        assert_eq!(vec![1, 2], td);
        assert_eq!(3, find_gap(&td));
    }

    #[test]
    fn test_named_workspace_with_gap() {
        let td = numbered_workspaces(vec![-1, 3, 1]);

        assert_eq!(2, find_gap(&td));
    }

    #[test]
    fn test_navigation_command() {
        assert_eq!(
            "workspace \"3: Chat\"",
            navigation_command("\"3: Chat\"", false, false)
        );
        assert_eq!(
            "move window to workspace number 3",
            navigation_command("number 3", true, false)
        );
        // one list, so a rename cannot land between the move and the switch
        assert_eq!(
            "move window to workspace \"3: Chat\"; workspace \"3: Chat\"",
            navigation_command("\"3: Chat\"", true, true)
        );
    }
}
