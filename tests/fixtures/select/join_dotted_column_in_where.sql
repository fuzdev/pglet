-- The same pair in a `WHERE` over the join, which is the **silent** member of the trio:
-- the reference reached a loose match on the target's own `v`, so the predicate simply
-- selected nothing where 18.6 returns the matching row. No error on either engine.
-- @setup
CREATE TABLE jdcwh_w (id INT, v TEXT);
CREATE TABLE jdcwh_s (id INT, "jdcwh_w.v" TEXT);
INSERT INTO jdcwh_w VALUES (1, 'a'), (2, 'b');
INSERT INTO jdcwh_s VALUES (1, 'srcval'), (2, 'srcval2');

-- @test
SELECT jdcwh_w.id FROM jdcwh_w JOIN jdcwh_s s ON s.id = jdcwh_w.id WHERE s."jdcwh_w.v" = 'srcval';
