-- The qualified spelling of `select/join_dotted_column_star.sql`'s collision: the pair
-- `("s", "jdc_w.v")` that the scan's own alias spells was never registered, so the
-- reference fell to a loose match and took the *target* relation's `v`. Silent, both
-- engines accepting.
-- @setup
CREATE TABLE jdcq_w (id INT, v TEXT);
CREATE TABLE jdcq_s (id INT, "jdcq_w.v" TEXT);
INSERT INTO jdcq_w VALUES (1, 'a'), (2, 'b');
INSERT INTO jdcq_s VALUES (1, 'srcval'), (2, 'srcval2');

-- @test
SELECT s."jdcq_w.v" AS got FROM jdcq_w JOIN jdcq_s s ON s.id = jdcq_w.id ORDER BY got;
