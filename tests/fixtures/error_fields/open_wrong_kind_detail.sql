-- The other wrong-kind wording: `validate_relation_kind`, reached by a scan, DML,
-- COPY, `CREATE INDEX … ON` or `REFERENCES` — `cannot open relation "x"` with the kind
-- in the DETAIL rather than the message.
-- @setup
CREATE TABLE t_wo2 (a INT);
CREATE INDEX t_wo2_idx ON t_wo2(a);

-- @test
-- @expect-error: INSERT naming an index
-- @no-profile-lock: the 42809 is raised by `get_table_or_err`'s relation-kind check, before the INSERT is planned at all — index selection is never reached on either profile
-- @expect-message
INSERT INTO t_wo2_idx VALUES (1);
