-- The catalog over an identity column the clause was added to: not nullable, as it was
-- declared before the clause, and an identity column.
-- @no-declared-types: the three columns are `yes_or_no` and `character_data` (base `varchar`, OID 1043) on 18.6, which pglet declares `text`
-- @setup
CREATE TABLE ai (c INT NOT NULL, v TEXT);
ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
-- @test
SELECT is_nullable, is_identity, identity_generation FROM information_schema.columns WHERE table_name = 'ai' AND column_name = 'c';
