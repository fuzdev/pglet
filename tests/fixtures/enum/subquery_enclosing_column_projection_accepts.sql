-- A sub-plan's projection can name a column of an **enclosing** query, and a descriptor
-- cannot carry that column's identity however it is built: `Expr::to_column_desc` runs
-- under the sub-plan's own scope, where the name does not resolve. The gate's subquery
-- leaf asks `certain_subquery_type_in` instead, which chains the enclosing levels
-- (`CertainLeafScope::column_enum`) — and the third entry here is the one the descriptor
-- alone answered `mood > text` for.
--
-- `(VALUES (1)) q` is the FROM that makes it reachable without a missing relation: the
-- sub-plan's own input registers `q` and nothing else, so `m` has no local claimant.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok'), (2, 'ok', 'ok'), (3, 'happy', 'zzz');

-- @test
SELECT id,
       (m IN (SELECT m FROM (VALUES (1)) q)) AS passthrough,
       (m IN (SELECT greatest(m, 'ok') FROM (VALUES (1)) q)) AS computed,
       (m > (SELECT m FROM (VALUES (1)) q)) AS scalar_operand
FROM t ORDER BY id;
