-- An ordering comparison of two enum constants is not trusted to decide a branch: a
-- member literal's cast carries no enum identity here, so it would compare by label
-- bytes (`'happy' < 'sad'`) where 18.6 compares by member position and keeps the `sum`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE em (a INT, b INT NOT NULL);
INSERT INTO em VALUES (1, 10), (2, 0);
-- @expect-error: division by zero
-- @test
SELECT ('happy'::mood < 'sad'::mood) OR sum(1/b) > 0 FROM em;
