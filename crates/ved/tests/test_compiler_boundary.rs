mod support;
use std::fs;

#[test]
fn imported_stack_references_resolve_after_merge_for_starlark_and_shorthand() {
    let root = std::env::temp_dir().join(format!("ved-stack-boundary-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("catalog/database")).unwrap();
    // Stack -> project dependency also needs the final merged symbol table.
    fs::write(root.join("catalog/database/Vedfile"), "services = [Service(name='db', image='postgres', ports=[5432], wait_for=['seed:started'])]\n").unwrap();
    let sources = [
        ("Vedfile", "app = App(name='boundary', version='1', env_files=[])\nStack.add('database')\napp.services.add(Service(name='seed', image='busybox'))\napp.services.add(Service(name='web', image='nginx', wait_for=['db:healthy']))\n"),
        ("spec.ved", "App boundary 1\nStack database\nService seed busybox\nService web nginx\nweb -> db\n"),
    ];
    for (name, source) in sources {
        let project = root.join(name.replace('.', "-"));
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join(name), source).unwrap();
        let output = support::command(env!("CARGO_BIN_EXE_ved"), &root)
            .args([
                "-d",
                project.to_str().unwrap(),
                "compile",
                "-f",
                name,
                "-t",
                "compose",
                "--output-dir",
                "out",
            ])
            .env("VED_STACKS_PATH", root.join("catalog"))
            .env_remove("VED_CONFIG_DIR")
            .env_remove("VED_WORK_DIR")
            .env_remove("VED_OUTPUT_DIR")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::remove_dir_all(root).unwrap();
}
