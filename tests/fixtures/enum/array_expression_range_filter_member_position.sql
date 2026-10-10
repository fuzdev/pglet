-- The other direction of `array_expression_filter_member_position`, and it separates a
-- different way: `> ARRAY['sad'::mood]` selects **two** rows by member position and
-- **none** by the label bytes, so a regression here is an empty result rather than a
-- swapped one.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'happy'), (2, 'sad'), (3, 'ok');

-- @test
SELECT id FROM t WHERE ARRAY[m] > ARRAY['sad'::mood] ORDER BY id;
