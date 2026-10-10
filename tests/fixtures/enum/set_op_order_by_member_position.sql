-- A set operation's output column has to carry the enum its two **branches share**: it
-- builds its descriptors fresh from the unified column types rather than propagating a
-- passthrough one, so nothing carries the enum across it by default. The enum
-- column rides through the set op and out of the wrapping derived table, and only the
-- outer `id`/`label` pair is projected — a bare enum column is not comparable by this
-- harness (`docs/enums.md` § Harness note).
--
-- The `ORDER BY` names the column rather than its position; `order_by_positional_member_position`
-- is the positional half, which resolves through a different arm entirely.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id, label FROM (
  (SELECT id, m, m::text AS label FROM t WHERE id < 3)
  UNION ALL
  (SELECT id, m, m::text AS label FROM t WHERE id >= 3)
) s
ORDER BY s.m, s.id;
