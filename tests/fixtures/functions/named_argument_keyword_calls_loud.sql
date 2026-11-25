-- A call spelled with one of 18.6's column-name keywords — `normalize`, a type name — is
-- its grammar's, which takes no named argument: a syntax error.
-- @expect-error: syntax error at or near "=>"
-- @test
SELECT normalize(a => 'x');
