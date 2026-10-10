-- **A control — do not trim it.** The single-relation reading of the quoted dotted column
-- the join fixtures collide on. A scan's column names are the user's own, so nothing
-- synthesizes a colliding spelling and both engines agree with or without the pair
-- keying — which is what identifies the join's addressing as the cause rather than the
-- column name.
-- @setup
CREATE TABLE jdsr_s (id INT, "jdsr_w.v" TEXT);
INSERT INTO jdsr_s VALUES (1, 'srcval'), (2, 'srcval2');

-- @test
SELECT "jdsr_w.v" AS got FROM jdsr_s ORDER BY got;
