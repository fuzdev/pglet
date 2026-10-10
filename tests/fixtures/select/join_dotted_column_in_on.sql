-- The same pair, read in the `ON` position — a different resolver (`JoinColumnMap`), so
-- a fix confined to the projection cannot reach it. The map is pair-keyed already; what
-- was forged is the pair handed to it, so `("s", "jdco_w.v")` was absent and this was
-- `column "s.jdco_w.v" does not exist` where 18.6 joins every pair.
-- @setup
CREATE TABLE jdco_w (id INT, v TEXT);
CREATE TABLE jdco_s (id INT, "jdco_w.v" TEXT);
INSERT INTO jdco_w VALUES (1, 'a'), (2, 'b');
INSERT INTO jdco_s VALUES (1, 'srcval'), (2, 'srcval2');

-- @test
SELECT jdco_w.id FROM jdco_w JOIN jdco_s s ON s."jdco_w.v" = 'srcval' ORDER BY jdco_w.id;
