-- `server_version_num` is the number a client feature-gates on and is byte-equal;
-- `server_version` carries the packaging suffix on 18.6 (`18.6 (Debian …)`) and not
-- here, so **its text is what this fixture never looks at** — only that it starts
-- with the same release, which is the one fact `pglet_sql::catalog::SERVER_VERSION`
-- and `version()`'s banner must agree on.
-- @test
SELECT
  current_setting('server_version_num') AS num,
  current_setting('server_version') LIKE '18.6%' AS same_release;
