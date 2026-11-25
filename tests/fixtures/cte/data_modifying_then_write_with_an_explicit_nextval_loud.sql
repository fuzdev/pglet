-- A sequence function written in either write draws in another order than 18.6's
-- row-by-row interleaving, whoever else draws the sequence, so it is refused (`0A000`;
-- 18.6 answers).
-- @setup
CREATE SEQUENCE dmq_s;
CREATE TABLE dmq (id INT, v INT);
CREATE TABLE dmq_d (id INT DEFAULT nextval('dmq_s'), tid INT);
-- @expect-error: two writes drawing one sequence
-- @test
WITH c AS (INSERT INTO dmq VALUES (nextval('dmq_s'), 1), (nextval('dmq_s'), 2) RETURNING id)
INSERT INTO dmq_d (tid) SELECT id FROM c;
