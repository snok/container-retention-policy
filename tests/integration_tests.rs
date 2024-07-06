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
