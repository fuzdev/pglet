-- The skip under another schema's qualifier comes after the statement is read whole: a
-- grammar error in the operation is still raised (`42601`), as 18.6's raw parse raises
-- it before any relation is looked up.
-- @expect-error: incomplete ADD COLUMN under an IF EXISTS skip
-- @test
ALTER TABLE IF EXISTS nosuchschema_atie42.atie42 ADD COLUMN;
