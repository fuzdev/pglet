-- @setup
CREATE TABLE wz (a INT, b INT);
INSERT INTO wz VALUES (1, 5), (1, 7), (2, 5);

-- The loud half of the same class: the select list drops `a` entirely, so the key
-- names nothing the Window's output carries and was `42703`. Its junk column is what
-- makes `row_number()` numbered over the input order and *then* sorted by a dropped
-- column — the shape a sort-below layout could not express.
-- @test
SELECT b, row_number() OVER () AS rn FROM wz ORDER BY a;
