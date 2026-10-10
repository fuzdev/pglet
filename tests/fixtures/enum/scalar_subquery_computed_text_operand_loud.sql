-- The control that makes `enum/scalar_subquery_computed_operand_accepts` a decision
-- rather than a waiver. The sub-plan reader is three-valued, and taking its middle state
-- at this door falls the pair back to the OID rule and **accepts** — measured turning
-- `DELETE FROM w WHERE m = (SELECT s::text FROM u)` into a silent row removal. Only the
-- reader's *decided* answer is taken here, so a `text` projection stays loud.
-- @expect-error: mood compared with a scalar subquery projecting text
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (id INT, m mood NOT NULL);
INSERT INTO w VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
CREATE TABLE u (id INT, s TEXT NOT NULL);
INSERT INTO u VALUES (1, 'ok');

-- @test
SELECT id FROM w WHERE m > (SELECT min(s) FROM u);
