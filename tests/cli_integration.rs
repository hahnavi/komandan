use clap::Parser;
use komandan::args::{Args, Commands, ProjectCommands};

#[test]
fn test_args_parsing_version_flag() {
    let args = Args::parse_from(["komandan", "--version"]);
    assert!(args.flags.version);
}

#[test]
fn test_args_parsing_project_init() {
    let args = Args::parse_from(["komandan", "project", "init", "my_dir"]);

    if let Some(Commands::Project(project_args)) = args.command {
        if let ProjectCommands::Init(init_args) = project_args.command {
            assert_eq!(init_args.directory, "my_dir");
        } else {
            panic!("Expected Init command");
        }
    } else {
        panic!("Expected Project command");
    }
}

#[test]
fn test_args_parsing_project_init_default() {
    let args = Args::parse_from(["komandan", "project", "init"]);

    if let Some(Commands::Project(project_args)) = args.command {
        if let ProjectCommands::Init(init_args) = project_args.command {
            assert_eq!(init_args.directory, ".");
        } else {
            panic!("Expected Init command");
        }
    } else {
        panic!("Expected Project command");
    }
}

#[test]
fn test_args_parsing_project_new() {
    let args = Args::parse_from(["komandan", "project", "new", "myproject"]);

    if let Some(Commands::Project(project_args)) = args.command {
        if let ProjectCommands::New(new_args) = project_args.command {
            assert_eq!(new_args.name, "myproject");
            assert!(new_args.dir.is_none());
        } else {
            panic!("Expected New command");
        }
    } else {
        panic!("Expected Project command");
    }
}

#[test]
fn test_args_parsing_project_new_with_dir() {
    let args = Args::parse_from([
        "komandan",
        "project",
        "new",
        "myproject",
        "--dir",
        "custom_dir",
    ]);

    if let Some(Commands::Project(project_args)) = args.command {
        if let ProjectCommands::New(new_args) = project_args.command {
            assert_eq!(new_args.name, "myproject");
            assert_eq!(new_args.dir, Some("custom_dir".to_string()));
        } else {
            panic!("Expected New command");
        }
    } else {
        panic!("Expected Project command");
    }
}

#[test]
fn test_args_parsing_with_main_file() {
    let args = Args::parse_from(["komandan", "script.lua"]);
    assert_eq!(args.main_file, Some("script.lua".to_string()));
}

#[test]
fn test_args_parsing_with_chunk() {
    let args = Args::parse_from(["komandan", "-e", "print('hello')"]);
    assert_eq!(args.chunk, Some("print('hello')".to_string()));
}

#[test]
fn test_args_parsing_flags() {
    let args = Args::parse_from([
        "komandan",
        "--dry-run",
        "--no-report",
        "--interactive",
        "--verbose",
        "--unsafe-lua",
    ]);

    assert!(args.flags.dry_run);
    assert!(args.flags.no_report);
    assert!(args.flags.interactive);
    assert!(args.flags.verbose);
    assert!(args.flags.unsafe_lua);
}

#[test]
fn test_args_parsing_short_flags() {
    let args = Args::parse_from(["komandan", "-d", "-n", "-i", "-v", "-u"]);

    assert!(args.flags.dry_run);
    assert!(args.flags.no_report);
    assert!(args.flags.interactive);
    assert!(args.flags.verbose);
    assert!(args.flags.unsafe_lua);
}

#[test]
fn test_args_parsing_combined_flags_and_main_file() {
    let args = Args::parse_from(["komandan", "--dry-run", "--interactive", "script.lua"]);

    assert_eq!(args.main_file, Some("script.lua".to_string()));
    assert!(args.flags.dry_run);
    assert!(args.flags.interactive);
}

#[test]
fn test_args_parsing_chunk_and_interactive() {
    let args = Args::parse_from(["komandan", "-e", "print('test')", "-i"]);

    assert_eq!(args.chunk, Some("print('test')".to_string()));
    assert!(args.flags.interactive);
}
