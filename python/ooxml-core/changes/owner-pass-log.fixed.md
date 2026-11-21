A document owner logs each pass before readers see the snapshot it made: a reader woken by a
new snapshot, or one reading the latest at once, finds that pass in the owner's statistics
(`stats`). Before, the pass was logged after the snapshot was published, and a reader could
read the log one pass behind (the flaky test `owner::commits_in_version_order_and_coalesced`).
