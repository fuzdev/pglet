-- A repeated **expression** part collapses as a repeated column part does — the same
-- membership test answers both — and the comment on `list_difference` in
-- `infer_arbiter_indexes` names exactly this case.
-- @setup
CREATE TABLE oarep (x TEXT, y INT);
CREATE UNIQUE INDEX oarep_lx ON oarep ((lower(x)), y);
INSERT INTO oarep VALUES ('A', 1);
INSERT INTO oarep VALUES ('a', 1) ON CONFLICT (lower(x), lower(x), y) DO NOTHING;

-- @test
SELECT x, y FROM oarep ORDER BY x;
