-- A default converting a `text` operand is a value conversion, run per row: 18.6 accepts
-- `DEFAULT 'q'::text::mood` at `CREATE TABLE` and records it, rendered `('q'::text)::mood`
-- — the cast written back as the one cast it is.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE md (id INT, m mood DEFAULT 'q'::text::mood);

-- @test
SELECT column_default::text AS d FROM information_schema.columns WHERE table_name = 'md' AND column_name = 'm';
