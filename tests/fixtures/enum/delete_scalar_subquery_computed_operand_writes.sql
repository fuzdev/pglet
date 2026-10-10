-- The `DELETE` polarity of `enum/scalar_subquery_computed_operand_accepts`: the
-- predicate a gate refused is the one a write takes, and the rows left behind are the
-- assertion. A refusal here leaves all three; an ordering by the label bytes rather than
-- the member positions leaves a different one.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (id INT, m mood NOT NULL);
INSERT INTO w VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
CREATE TABLE u (id INT, m mood NOT NULL);
INSERT INTO u VALUES (1, 'ok');

-- @test
DELETE FROM w WHERE m < (SELECT min(m) FROM u);
SELECT id, m::text AS m_text FROM w ORDER BY id;
