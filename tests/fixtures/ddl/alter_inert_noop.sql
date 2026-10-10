-- A genuinely-inert pg_dump-preamble ALTER clause still no-ops in both engines
-- (the RENAME fail-loud must not break the tolerated-clause path).
-- @setup
CREATE TABLE alter_inert (a INT);
INSERT INTO alter_inert VALUES (1), (2);
ALTER TABLE alter_inert SET (fillfactor = 70);

-- @test
SELECT count(*) FROM alter_inert;
