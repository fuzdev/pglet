-- A `DISTINCT ON` **expression** key — `DISTINCT ON (m > 'ok')` — is evaluated at the
-- `DistinctOn` node's own scope, which is a separate eval entry from the `Sort` above it
-- and from the scan below. `distinct_on_member_position`'s key is a plain `INT`, so it
-- reaches this scope with nothing to order.
--
-- `sad` and `ok` are both `<= ok` and `happy` is not, so member position keeps ids 1 and
-- 3; the label bytes make `happy` and `ok` both false and `sad` true, keeping 1 and 2.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id FROM (
  SELECT DISTINCT ON (m > 'ok') id, m FROM t ORDER BY (m > 'ok'), id
) s
ORDER BY id;
