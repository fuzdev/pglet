-- Each serial spelling added by `ADD COLUMN` is declared `NOT NULL`, over a table that
-- already holds rows.
-- @no-declared-types: `column_name` is `sql_identifier` (base `name`, OID 19) and `is_nullable` is `yes_or_no` (base `varchar`, OID 1043) on 18.6, which pglet declares `text`
-- @setup
CREATE TABLE acs (v TEXT);
INSERT INTO acs VALUES ('a'), ('b');
ALTER TABLE acs ADD COLUMN a SERIAL;
ALTER TABLE acs ADD COLUMN b BIGSERIAL;
ALTER TABLE acs ADD COLUMN c SMALLSERIAL;
ALTER TABLE acs ADD COLUMN d SERIAL4;
ALTER TABLE acs ADD COLUMN e SERIAL8;
ALTER TABLE acs ADD COLUMN f SERIAL2;
ALTER TABLE acs ADD COLUMN IF NOT EXISTS g SERIAL;
-- @test
SELECT column_name, is_nullable FROM information_schema.columns WHERE table_name = 'acs' ORDER BY ordinal_position;
