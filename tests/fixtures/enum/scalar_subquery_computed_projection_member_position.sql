-- `EDGES.md` row 107's own shape: a scalar subquery projecting an **expression** rather
-- than a passthrough column, compared against a member. Its descriptor carried no enum,
-- so the comparison ordered by the label bytes — `min(m)` is `ok`, and `ok > sad` is
-- `t` by member position and `f` by bytes. The descriptor is built by a derivation now,
-- so both readers of it — this ordering one and the gate — answer from the projection.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE u (id INT, m mood NOT NULL);
INSERT INTO u VALUES (1, 'ok'), (2, 'happy');

-- @test
SELECT
  (SELECT min(m) FROM u) > 'sad' AS min_gt_sad,
  (SELECT max(m) FROM u) < 'ok' AS max_lt_ok,
  (SELECT COALESCE(m, 'sad') FROM u WHERE id = 1) > 'sad' AS coalesce_gt_sad,
  (SELECT greatest(m, 'sad') FROM u WHERE id = 1) BETWEEN 'sad' AND 'happy' AS greatest_btw;
