-- `DROP EXTENSION IF EXISTS` over a name nothing installed is a skip on both
-- engines (18.6 emits a NOTICE and reports `DROP EXTENSION`). The trailing `SELECT`
-- keeps the fixture an assertion rather than an empty result.
-- @test
DROP EXTENSION IF EXISTS nosuch;
DROP EXTENSION IF EXISTS nosuch1, nosuch2 CASCADE;
SELECT 1 AS ok;
