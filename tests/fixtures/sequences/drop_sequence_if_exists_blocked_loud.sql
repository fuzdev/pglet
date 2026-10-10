-- `IF EXISTS` waives only the *missing* case. A dependent that blocks is reached long
-- after the name resolved, so the skip never fires.
-- @setup
CREATE TABLE dsieb_t (id SERIAL, b INT);
-- @test
DROP SEQUENCE IF EXISTS dsieb_t_id_seq;
-- @expect-error: a column default still blocks under `IF EXISTS` (2BP01)
