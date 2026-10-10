-- `DROP TYPE … CASCADE` clears a column `DEFAULT` converting to the enum and keeps the
-- column: an insert that omits it stores NULL.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE md (s TEXT DEFAULT ('ok'::text)::mood, n INT);

-- @test
DROP TYPE mood CASCADE;
INSERT INTO md (n) VALUES (1);
SELECT s, n, (SELECT column_default::text FROM information_schema.columns
              WHERE table_name = 'md' AND column_name = 's') AS dflt
FROM md;
