-- An `unknown` literal in the recursive term is converted to the seed's enum, so a
-- label it lacks is `22P02` whatever the rows — this term yields none.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (m mood);
INSERT INTO w VALUES ('sad'), ('happy');
CREATE TABLE tx (v TEXT);
-- @expect-error: non-member literal converted to the seed enum (22P02)
-- @test
WITH RECURSIVE r(x) AS (SELECT m FROM w UNION ALL SELECT 'zzz' FROM r WHERE false)
SELECT * FROM r;
