-- The accept control for the `RETURNING` half of the `INSERT` clause re-walk: a
-- correlated subquery that legitimately names the target relation still resolves. The
-- gate that closed `insert/returning_subquery_key_binds_to_nothing_loud.sql` runs over
-- exactly this statement, so a fix that closed it by over-rejecting — by handing the
-- nested level a scope narrower than the clause's own namespace, or by declining to
-- register the target at all — reddens here.
-- @setup
CREATE TABLE rnt_target (v INT);
CREATE TABLE rnt_src (b INT);
INSERT INTO rnt_src VALUES (5), (7);

-- @test
INSERT INTO rnt_target VALUES (5)
  RETURNING v, (SELECT count(*) FROM rnt_src WHERE rnt_src.b = rnt_target.v) AS hits;
