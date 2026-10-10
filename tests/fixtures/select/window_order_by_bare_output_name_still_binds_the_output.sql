-- @setup
CREATE TABLE wz (a INT, b INT);
INSERT INTO wz VALUES (1, 5), (1, 7), (2, 5);

-- The accept-polarity control for the two above: a key that **is** a whole bare name
-- matching an output column does take the alias, which is the half of
-- `findTargetlistEntrySQL92` that survives. Identical statement to the nested-name
-- fixture but for the `+ 0`, and it answers the other order.
-- @test
SELECT b AS a, row_number() OVER () AS rn FROM wz ORDER BY a;
