-- The other arm the DDL-time rulebook was missing. A simple `CASE`'s label is Postgres's
-- `x = v`, so it takes the operator refusal and not a fold's.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
CREATE TABLE ck (m mood NOT NULL, s TEXT NOT NULL,
                 CHECK ((CASE m WHEN s THEN 1 ELSE 0 END) = 0));
