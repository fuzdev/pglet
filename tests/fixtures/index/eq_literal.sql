-- Index-accelerated point read: CREATE INDEX + `WHERE col = <literal>` over a
-- column with duplicate values must return identical rows to real Postgres. The
-- planner emits an IndexScan; the differential harness only sees the rows, which
-- must match the full-scan result byte-for-byte.
-- @profile-lock
-- @setup
CREATE TABLE idx_items (id INT, k INT, name TEXT);
INSERT INTO idx_items VALUES
  (1, 10, 'a'),
  (2, 20, 'b'),
  (3, 10, 'c'),
  (4, 30, 'd'),
  (5, 10, 'e');
CREATE INDEX idx_items_k ON idx_items(k);
-- @test
SELECT id, k, name FROM idx_items WHERE k = 10 ORDER BY id;
