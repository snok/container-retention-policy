container-retention-policy on  v3-develop [✘!+] via 🐳 desktop-linux is 📦 v2.0.0 via 🐍 v3.12.1 via 🦀 v1.78.0
❯ time RUST_LOG=container_retention_policy=info ./target/release/container-retention-policy \
--account snok \
--token redacted \
--tag-selection both \
--image-names "container-retention-policy*"  \
--image-tags "!latest !test-1*" \
--shas-to-skip "" \
--keep-at-least 2 \
--timestamp-to-use updated-at \
--cut-off 1s \
--dry-run true
2024-05-28T13:30:09.489974Z  INFO container_retention_policy: Found 1 package(s) for the "snok" organization
2024-05-28T13:30:09.490067Z  INFO container_retention_policy: 1/1 package names matched the `package-name` filters
2024-05-28T13:30:09.866345Z  INFO container_retention_policy: ✓ package version matched all `image-tags` filters package_name="container-retention-policy" package_version_id=221935776 tags=["test-5-3", "test-5-2", "test-5-1", "test-5"]
2024-05-28T13:30:09.866432Z  INFO container_retention_policy: ✓ package version matched all `image-tags` filters package_name="container-retention-policy" package_version_id=221935602 tags=["test-4-3", "test-4-2", "test-4-1", "test-4"]
2024-05-28T13:30:09.866450Z  INFO container_retention_policy: ✓ package version matched all `image-tags` filters package_name="container-retention-policy" package_version_id=221935421 tags=["test-3-3", "test-3-2", "test-3-1", "test-3"]
2024-05-28T13:30:09.866470Z  INFO container_retention_policy: ✓ package version matched all `image-tags` filters package_name="container-retention-policy" package_version_id=221935339 tags=["test-2-3", "test-2-2", "test-2-1", "test-2"]
2024-05-28T13:30:09.866624Z  INFO container_retention_policy: ✕ package version matched a negative `image-tags` filter package_name="container-retention-policy" package_version_id=221935206 tags=["test-1-3", "test-1-2", "test-1-1", "test-1"]
2024-05-28T13:30:09.866665Z  INFO container_retention_policy: Kept 2 of the 2 package versions requested by the `keep-at-least` setting package_name="container-retention-policy"
2024-05-28T13:30:09.866690Z  INFO container_retention_policy: Selected 2 tagged and 16 untagged package versions for deletion
2024-05-28T13:30:09.866758Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:<untagged> package_version=221935772
2024-05-28T13:30:09.866796Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:<untagged> package_version=221935763
2024-05-28T13:30:09.866870Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:<untagged> package_version=221935417
2024-05-28T13:30:09.870516Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:<untagged> package_version=221935412
2024-05-28T13:30:09.866909Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:<untagged> package_version=221935336
2024-05-28T13:30:09.866936Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:<untagged> package_version=221935194
2024-05-28T13:30:09.872139Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:<untagged> package_version=221935583
2024-05-28T13:30:09.866972Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:<untagged> package_version=221927611
2024-05-28T13:30:09.866987Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:<untagged> package_version=221927608
2024-05-28T13:30:09.867007Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:<untagged> package_version=221927603
2024-05-28T13:30:09.867023Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:<untagged> package_version=221919161
2024-05-28T13:30:09.867049Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:<untagged> package_version=221919147
2024-05-28T13:30:09.870450Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:<untagged> package_version=221918492
2024-05-28T13:30:09.870489Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:test-2-3 package_version=221935339
2024-05-28T13:30:09.875828Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:test-2-2 package_version=221935339
2024-05-28T13:30:09.866871Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:<untagged> package_version=221935595
2024-05-28T13:30:09.870539Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:test-3-3 package_version=221935421
2024-05-28T13:30:09.872096Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:<untagged> package_version=221935334
2024-05-28T13:30:09.875845Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:test-2-1 package_version=221935339
2024-05-28T13:30:09.875882Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:test-3-2 package_version=221935421
2024-05-28T13:30:09.880704Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:test-3-1 package_version=221935421
2024-05-28T13:30:09.880715Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:test-3 package_version=221935421
2024-05-28T13:30:09.866953Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:<untagged> package_version=221935188
2024-05-28T13:30:09.880684Z  INFO container_retention_policy::client: dry-run: Would have deleted container-retention-policy:test-2 package_version=221935339

________________________________________________________
Executed in  900.87 millis    fish           external
usr time   20.75 millis    0.23 millis   20.52 millis
sys time   13.94 millis    1.25 millis   12.69 millis


container-retention-policy on  v3-develop [✘!+] via 🐳 desktop-linux is 📦 v2.0.0 via 🐍 v3.12.1 via 🦀 v1.78.0
❯ time RUST_LOG=container_retention_policy=info ./target/release/container-retention-policy \
--account snok \
--token redacted \
--tag-selection both \
--image-names "container-retention-policy*"  \
--image-tags "!latest !test-1*" \
--shas-to-skip "" \
--keep-at-least 2 \
--timestamp-to-use updated-at \
--cut-off 1s \
--dry-run false
2024-05-28T13:30:21.007923Z  INFO container_retention_policy: Found 1 package(s) for the "snok" organization
2024-05-28T13:30:21.007985Z  INFO container_retention_policy: 1/1 package names matched the `package-name` filters
2024-05-28T13:30:21.334673Z  INFO container_retention_policy: ✓ package version matched all `image-tags` filters package_name="container-retention-policy" package_version_id=221935776 tags=["test-5-3", "test-5-2", "test-5-1", "test-5"]
2024-05-28T13:30:21.334763Z  INFO container_retention_policy: ✓ package version matched all `image-tags` filters package_name="container-retention-policy" package_version_id=221935602 tags=["test-4-3", "test-4-2", "test-4-1", "test-4"]
2024-05-28T13:30:21.334781Z  INFO container_retention_policy: ✓ package version matched all `image-tags` filters package_name="container-retention-policy" package_version_id=221935421 tags=["test-3-3", "test-3-2", "test-3-1", "test-3"]
2024-05-28T13:30:21.334799Z  INFO container_retention_policy: ✓ package version matched all `image-tags` filters package_name="container-retention-policy" package_version_id=221935339 tags=["test-2-3", "test-2-2", "test-2-1", "test-2"]
2024-05-28T13:30:21.334965Z  INFO container_retention_policy: ✕ package version matched a negative `image-tags` filter package_name="container-retention-policy" package_version_id=221935206 tags=["test-1-3", "test-1-2", "test-1-1", "test-1"]
2024-05-28T13:30:21.335009Z  INFO container_retention_policy: Kept 2 of the 2 package versions requested by the `keep-at-least` setting package_name="container-retention-policy"
2024-05-28T13:30:21.335032Z  INFO container_retention_policy: Selected 2 tagged and 16 untagged package versions for deletion
2024-05-28T13:30:21.659586Z  INFO container_retention_policy::client: Deleted container-retention-policy:<untagged> package_version_id=221935583
2024-05-28T13:30:21.683804Z  INFO container_retention_policy::client: Deleted container-retention-policy:<untagged> package_version_id=221935417
2024-05-28T13:30:21.711312Z  INFO container_retention_policy::client: Deleted container-retention-policy:test-2-3 package_version_id=221935339
2024-05-28T13:30:21.711361Z  INFO container_retention_policy::client: Deleted container-retention-policy:test-2-2 package_version_id=221935339
2024-05-28T13:30:21.711365Z  INFO container_retention_policy::client: Deleted container-retention-policy:test-2-1 package_version_id=221935339
2024-05-28T13:30:21.711369Z  INFO container_retention_policy::client: Deleted container-retention-policy:test-2 package_version_id=221935339
2024-05-28T13:30:21.713884Z  INFO container_retention_policy::client: Deleted container-retention-policy:<untagged> package_version_id=221927611
2024-05-28T13:30:21.714856Z  INFO container_retention_policy::client: Deleted container-retention-policy:<untagged> package_version_id=221935763
2024-05-28T13:30:21.714956Z  INFO container_retention_policy::client: Deleted container-retention-policy:<untagged> package_version_id=221935334
2024-05-28T13:30:21.717372Z  INFO container_retention_policy::client: Deleted container-retention-policy:<untagged> package_version_id=221935412
2024-05-28T13:30:21.719187Z  INFO container_retention_policy::client: Deleted container-retention-policy:<untagged> package_version_id=221918492
2024-05-28T13:30:21.724613Z  INFO container_retention_policy::client: Deleted container-retention-policy:<untagged> package_version_id=221919161
2024-05-28T13:30:21.725512Z  INFO container_retention_policy::client: Deleted container-retention-policy:<untagged> package_version_id=221935194
2024-05-28T13:30:21.725591Z  INFO container_retention_policy::client: Deleted container-retention-policy:<untagged> package_version_id=221927608
2024-05-28T13:30:21.726282Z  INFO container_retention_policy::client: Deleted container-retention-policy:<untagged> package_version_id=221935772
2024-05-28T13:30:21.727003Z  INFO container_retention_policy::client: Deleted container-retention-policy:test-3-3 package_version_id=221935421
2024-05-28T13:30:21.727014Z  INFO container_retention_policy::client: Deleted container-retention-policy:test-3-2 package_version_id=221935421
2024-05-28T13:30:21.727018Z  INFO container_retention_policy::client: Deleted container-retention-policy:test-3-1 package_version_id=221935421
2024-05-28T13:30:21.727022Z  INFO container_retention_policy::client: Deleted container-retention-policy:test-3 package_version_id=221935421
2024-05-28T13:30:21.727889Z  INFO container_retention_policy::client: Deleted container-retention-policy:<untagged> package_version_id=221935336
2024-05-28T13:30:21.728281Z  INFO container_retention_policy::client: Deleted container-retention-policy:<untagged> package_version_id=221935595
2024-05-28T13:30:21.733786Z  INFO container_retention_policy::client: Deleted container-retention-policy:<untagged> package_version_id=221935188
2024-05-28T13:30:21.740847Z  INFO container_retention_policy::client: Deleted container-retention-policy:<untagged> package_version_id=221919147
2024-05-28T13:30:21.752764Z  INFO container_retention_policy::client: Deleted container-retention-policy:<untagged> package_version_id=221927603

________________________________________________________
Executed in    1.28 secs      fish           external
usr time   65.97 millis   17.28 millis   48.69 millis
sys time   38.64 millis    4.50 millis   34.14 millis
