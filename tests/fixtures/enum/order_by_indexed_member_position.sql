-- The same `ORDER BY` as `order_by_member_position`, over an **indexed** enum column.
-- The B-tree key encoder has no enum arm — it encodes the label's bytes — so an ordered
-- index scan would answer the label order while the row beside it answers the member
-- order. Index selection declines an enum key column for exactly that reason, which is
-- why this carries **no** `@profile-lock`: both profiles now plan the same
-- `Sort(Scan)`, and leaving the tag off makes a revert red the lock run's plan-derived
-- completeness check as well as this run's rows (the `index/gin_method_*` precedent).
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
CREATE INDEX t_m_idx ON t (m);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id, m::text AS label FROM t ORDER BY m;
