-- 18.6 has two wordings for a wrong-kind relation and they come from different
-- functions. This is `DropErrorMsgWrongType`: `"x" is not a table`, with the removal
-- for the kind it *actually* is in the HINT.
-- @setup
CREATE TABLE t_wo (a INT);
CREATE INDEX t_wo_idx ON t_wo(a);

-- @test
-- @expect-error: DROP TABLE naming an index
-- @no-profile-lock: the 42809 is raised by `resolve_relation_for_drop`'s kind check on a DDL statement — a catalog name lookup with no plan and no index read on either profile
-- @expect-message
DROP TABLE t_wo_idx;
