-- The accept side of the `ON CONFLICT` arbiter match and the `DO UPDATE SET` target,
-- both of which are exact comparisons: the fold
-- has to reach the `CREATE TABLE` key column, the arbiter list, the `SET` target and
-- the `EXCLUDED` qualifier for this to resolve at all. The reject side is a Rust test
-- (`case_folding.rs`), because pglet answers `42P10` where 18.6 answers `42703` and an
-- `@expect-error` fixture requires the SQLSTATEs to agree.
-- @setup
CREATE TABLE ocfold (N INT PRIMARY KEY, V INT);
INSERT INTO ocfold VALUES (1, 1);

-- @test
INSERT INTO ocfold VALUES (1, 2) ON CONFLICT (n) DO UPDATE SET V = Excluded.v RETURNING n, v;
