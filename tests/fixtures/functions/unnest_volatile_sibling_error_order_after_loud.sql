-- The mirror of the order case: a failing non-volatile sibling before a volatile one is
-- the error reported, the list's own order.
-- @setup
CREATE TABLE uo (i INT);
INSERT INTO uo VALUES (1), (2);
-- @test
-- @expect-error: integer out of range
SELECT unnest(ARRAY[1]), (SELECT i + 2147483647 FROM uo WHERE i = 2), (random() * 0)::int / 0;
