-- The derived-table half of `cte_order_by_member_position`: a different plan node
-- (`SubqueryScan`) reaching the same descriptor propagation.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id, m::text AS label FROM (SELECT id, m FROM t) d ORDER BY m;
