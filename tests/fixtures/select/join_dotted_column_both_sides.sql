-- Both relations carry the same quoted dotted column, which is what puts the resolution
-- in the descriptor **list** rather than in the spelling: with one such column the loose
-- match happened to land on it, and with two it took the first — the left side's — where
-- 18.6 answers the side the qualifier names. Silent, both engines accepting.
--
-- Its opposite polarity is the same statement naming the left relation, green on both
-- engines throughout: `select/join_dotted_column_both_sides_left.sql`.
-- @setup
CREATE TABLE jdbs_l (id INT, "jdbs_x.v" TEXT);
CREATE TABLE jdbs_r (id INT, "jdbs_x.v" TEXT);
INSERT INTO jdbs_l VALUES (1, 'left'), (2, 'left2');
INSERT INTO jdbs_r VALUES (1, 'srcval'), (2, 'srcval2');

-- @test
SELECT s."jdbs_x.v" AS got FROM jdbs_l JOIN jdbs_r s ON s.id = jdbs_l.id ORDER BY got;
