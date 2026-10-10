-- `RETURNING *` over an `UPDATE … FROM` that matches **no row**.
--
-- The statement answers zero rows on both engines and still describes the **full**
-- widened column list — 18.6's `RowDescription` carries the target's columns and the
-- source's whether or not anything matched, and so must pglet's. That is the one thing a
-- row-count-dependent implementation cannot do: a namespace derived from the joined rows
-- rather than from the statement has nothing to describe here, and a `Describe` before
-- `Execute` (which is what a driver's `prepare()` issues) has no rows to derive it from
-- at all.
--
-- With no values to compare, the declared column types *are* the comparison, on both
-- protocol legs. The column names are distinct across the two relations so the harness's
-- name-keyed rows do not collapse the source's away.
-- @setup
CREATE TABLE rnm_t (t_id INT, t_v TEXT);
CREATE TABLE rnm_s (s_id INT, s_v TEXT, s_k BIGINT);
INSERT INTO rnm_t VALUES (1, 'TARGET');
INSERT INTO rnm_s VALUES (1, 'SOURCE', 7);

-- @test
UPDATE rnm_t SET t_id = 1 FROM rnm_s s WHERE s.s_id = 999 RETURNING *;
