use assert_cmd::Command;

#[test]
fn parse_input() {
    let args_permutations = vec![
        vec![
            "--account=user",
            "--token=ghs_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT",
            "--image-names=foo",
            "--image-tags=one",
            "--shas-to-skip=",
            "--keep-n-most-recent=0",
            "--tag-selection=tagged",
            "--timestamp-to-use=updated_at",
            "--cut-off=1w",
            "--dry-run=true",
        ],
        vec![
            "--account=acme",
            "--token=ghp_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT",
            "--image-names=\"foo bar\"",
            "--image-tags=\"one two\"",
            "--shas-to-skip=",
            "--keep-n-most-recent=10",
            "--tag-selection=untagged",
            "--timestamp-to-use=created_at",
            "--cut-off=1d",
            "--dry-run=true",
        ],
        vec![
            "--account=foo",
            "--token=ghp_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT",
            "--image-names=\"foo, bar\"",
            "--image-tags=\"one, two\"",
            "--shas-to-skip=''",
            "--keep-n-most-recent=999",
            "--tag-selection=both",
            "--timestamp-to-use=updated_at",
            "--cut-off=1h",
            "--dry-run=true",
        ],
        vec![
            "--account=$;\u{b}\n₭↭",
            "--token=ghp_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT",
            "--image-names=\"foo, bar\"",
            "--image-tags=\"one, two\"",
            "--shas-to-skip=''",
            "--keep-n-most-recent=2",
            "--tag-selection=both",
            "--timestamp-to-use=updated_at",
            "--cut-off=1h",
            "--dry-run=true",
        ],
    ];

    for args in args_permutations {
        let mut cmd = Command::cargo_bin(env!("CARGO_PKG_NAME")).expect("Failed to load binary");

        cmd.env("CRP_TEST", "true").args(args).assert().success();
    }
}

#[test]
fn url_env_parsing() {
    let arbitrary_set_of_inputs = vec![
        "--account=user",
        "--token=ghs_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT",
        "--image-names=foo",
        "--image-tags=one",
        "--shas-to-skip=",
        "--keep-n-most-recent=0",
        "--tag-selection=tagged",
        "--timestamp-to-use=updated_at",
        "--cut-off=1w",
        "--dry-run=true",
    ];
    let mut cmd = Command::cargo_bin(env!("CARGO_PKG_NAME")).expect("Failed to load binary");

    let output = cmd
        .env("CRP_TEST", "true")
        .env("RUST_LOG", "container_retention_policy=info")
        .env("GITHUB_SERVER_URL", "http://127.0.0.1:8000")
        .env("GITHUB_API_URL", "http://127.0.0.1:8001")
        .args(arbitrary_set_of_inputs)
        .output()
        .expect("Failed to execute command");

    let stderr = String::from_utf8_lossy(output.stderr.as_slice()).to_string();

    assert!(stderr.contains("Using provided GitHub server url: http://127.0.0.1:8000"));
    assert!(stderr.contains("Using provided GitHub API url: http://127.0.0.1:8001"));
}
