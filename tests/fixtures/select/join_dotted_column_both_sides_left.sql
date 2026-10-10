-- The accept-polarity control for `select/join_dotted_column_both_sides.sql`: naming the
-- **left** relation answered correctly on both engines before and after, because the
-- loose match took the first descriptor and the first descriptor was the right one. It is
-- what shows the sibling's divergence is the resolution and not the column name.
-- @setup
CREATE TABLE jdbsl_l (id INT, "jdbsl_x.v" TEXT);
CREATE TABLE jdbsl_r (id INT, "jdbsl_x.v" TEXT);
INSERT INTO jdbsl_l VALUES (1, 'left'), (2, 'left2');
INSERT INTO jdbsl_r VALUES (1, 'srcval'), (2, 'srcval2');

-- @test
SELECT jdbsl_l."jdbsl_x.v" AS got FROM jdbsl_l JOIN jdbsl_r s ON s.id = jdbsl_l.id ORDER BY got;
