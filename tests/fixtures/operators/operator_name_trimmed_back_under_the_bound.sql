-- The length check runs *after* the trailing-`+`/`-` trim, so a 65-character run that
-- trims to 63 is not too long — it is the 63-character operator, refused at analysis.
-- Reading the length before the trim would make this `42601`.
-- @test
-- @expect-error: trimmed to 63 characters, so analysis refuses it (42883)
-- @expect-message
SELECT 1 ***************************************************************++ 1;
