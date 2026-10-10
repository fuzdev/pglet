-- A **negative** literal is an index probe, which it was not before the unary-minus fold:
-- `-1` parsed to an `Expr::Negate`, and `is_row_independent` accepts only a bare literal /
-- `$N` / bound value, so the planner declined the index and row-filtered. Folded into the
-- constant (Postgres's `doNegate`), it probes exactly as `1` does — an `IndexScan` on the
-- accelerated profile against `Filter(Scan)` on the stripped one, which is why this
-- carries the lock: an index may change only the speed, so both profiles must answer
-- these rows. `+1` is deliberately still declined (unary plus stays a node on both
-- engines); `planner::tests`' `a_folded_negative_literal_is_a_probe_where_unary_plus_is_not`
-- pins the pair as plan shapes, which the differential corpus cannot see.
-- @profile-lock
-- @setup
CREATE TABLE ixneg (id INT, k INT, name TEXT);
INSERT INTO ixneg VALUES
  (1, -10, 'a'),
  (2, 20, 'b'),
  (3, -10, 'c'),
  (4, -30, 'd'),
  (5, 0, 'e');
CREATE INDEX ixneg_k ON ixneg(k);
-- @test
SELECT id, k, name FROM ixneg WHERE k = -10 ORDER BY id;
