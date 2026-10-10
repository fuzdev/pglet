-- The **bare** spelling of the same collision, and the one that was loud: two output
-- descriptors were literally named `jdcb_w.v` — the target's own `v` re-synthesized, and
-- the source's quoted column — so a bare reference was `42702` where 18.6 resolves the
-- source's column, the only one whose *name* is `jdcb_w.v`.
-- @setup
CREATE TABLE jdcb_w (id INT, v TEXT);
CREATE TABLE jdcb_s (id INT, "jdcb_w.v" TEXT);
INSERT INTO jdcb_w VALUES (1, 'a'), (2, 'b');
INSERT INTO jdcb_s VALUES (1, 'srcval'), (2, 'srcval2');

-- @test
SELECT "jdcb_w.v" AS got FROM jdcb_w JOIN jdcb_s s ON s.id = jdcb_w.id ORDER BY got;
