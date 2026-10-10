-- The array node unifies **every** element, not just the first. Here element 0 is a cast
-- (whose enum identity the parser erased) and element 1 is the enum column, so an arm
-- that read the first element alone would find no enum and sort by the label bytes —
-- while element 0 compares equal on every row and element 1 decides the whole order.
--
-- The separating property is a relation between the elements, not a value in either:
-- `order_by_array_expression_member_position` is green under a first-element rule
-- because its one element is the column.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'happy'), (2, 'sad'), (3, 'ok');

-- @test
SELECT id, m::text AS label FROM t ORDER BY ARRAY['ok'::mood, m], id;
