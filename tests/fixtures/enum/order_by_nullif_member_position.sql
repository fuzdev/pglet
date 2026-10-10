-- `nullif(a, b)`'s type is 18.6's `exprType(linitial(args))` — the first argument
-- **after** `make_op` coerced the pair to a common type (`transformAExprNullIf`,
-- `parse_expr.c`) — so an enum in *either* argument types the call. pglet unifies both
-- arguments, which reproduces that without a coercion pass. The `ok` row becomes NULL
-- and sorts last on both engines, so the assertion is the order of the other two.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id FROM t ORDER BY nullif(m, 'ok'), id;
