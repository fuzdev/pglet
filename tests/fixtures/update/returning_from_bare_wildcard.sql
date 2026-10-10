-- `UPDATE t SET … FROM s WHERE … RETURNING *` — the bare wildcard spans the whole
-- `RETURNING` namespace, which is the statement's own FROM namespace: the target's
-- columns, then the source's, in FROM order.
--
-- pglet used to expand `*` against the target alone and answer two columns — no error,
-- plausible column names, and a client reading the row by name quietly missing half its
-- result set — and then, for a while, refused the spelling outright (`0A000`) because
-- the executor dropped the join image before the projection. It keeps it now, so the
-- statement answers what 18.6 answers.
--
-- **The column names here are deliberately distinct across the two relations.** The
-- harness keys its recorded rows by column name, so 18.6's own duplicate `id`/`v` pair
-- would collapse and a build answering three columns would compare equal to one
-- answering five. The count, the order, and which relation each duplicate came from are
-- asserted where they are visible: `crates/pglet/tests/returning_from_namespace.rs`.
-- What this file adds over that unit test is the **declared column types**, compared
-- live against 18.6 on both protocol legs — a source column's type reaches a client
-- through the `RowDescription`, and the two walkers that produce it are separate code.
--
-- The two tables differ in arity (2 against 3) and in every value, so a wildcard
-- expanded against the wrong relation shows in the shape *and* in the data.
-- @setup
CREATE TABLE rbw_t (t_id INT, t_v TEXT);
CREATE TABLE rbw_s (s_id INT, s_v TEXT, s_k BIGINT);
INSERT INTO rbw_t VALUES (1, 'TARGET');
INSERT INTO rbw_s VALUES (1, 'SOURCE', 7);

-- @test
UPDATE rbw_t SET t_id = 1 FROM rbw_s s WHERE s.s_id = rbw_t.t_id RETURNING *;
