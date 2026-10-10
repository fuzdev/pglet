-- The enum rides the **column descriptor**, and a projecting node propagates the whole
-- descriptor for a passthrough column — so a CTE reference orders by member position
-- without a second derivation. Paired with `derived_table_order_by_member_position`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
WITH w AS (SELECT id, m FROM t) SELECT id, m::text AS label FROM w ORDER BY m;
