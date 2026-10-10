-- An enum under a `COALESCE` is still that enum — 18.6 gives `COALESCE(m, 'sad')` the
-- type `mood` by ordinary type unification, so the sort key orders by member position.
-- pglet has to derive that identity, and the walk that does it
-- (`executor::expr`'s `expr_enum_with`) reached a column reference and three
-- value-preserving wrappers over one, with a `_ => None` catch-all: every kind that
-- unifies its operands' types fell through it and sorted by label bytes. The match is exhaustive now, so the next such
-- kind is a compile error rather than another silent wrong order.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy'), (4, NULL);

-- @test
SELECT id, COALESCE(m, 'sad')::text AS label FROM t ORDER BY COALESCE(m, 'sad'), id;
