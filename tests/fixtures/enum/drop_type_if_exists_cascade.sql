-- `DROP TYPE IF EXISTS … CASCADE` is the teardown idiom: the first drops the dependent and
-- the type, the second skips.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE mi (s TEXT CHECK (s::mood IS NOT NULL));

-- @test
DROP TYPE IF EXISTS mood CASCADE;
DROP TYPE IF EXISTS mood CASCADE;
INSERT INTO mi VALUES ('zzz');
SELECT s FROM mi;
