-- `DESC` reverses the member order, not the label order — the direction is applied
-- after the comparison, so this is the exact reverse of `order_by_member_position`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id, m::text AS label FROM t ORDER BY m DESC;
