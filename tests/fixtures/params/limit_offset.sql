-- Bound `LIMIT $1 OFFSET $2` (bigint params). These are dropped at plan time
-- unless the planner keeps the placeholder for bind to resolve.
-- @setup
CREATE TABLE p_page (id int4);
INSERT INTO p_page (id) VALUES (1), (2), (3), (4), (5);
-- @param $1 int8 2
-- @param $2 int8 1
-- @test
SELECT id FROM p_page ORDER BY id LIMIT $1 OFFSET $2;
