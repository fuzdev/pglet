-- The other half alone, refused where the escape is read.
-- @test
-- @expect-error: a second surrogate half with no first
-- @no-error-fields: pglet's `P` is the string literal's own start where 18.6's is the
-- offending escape inside it (and, for an unpaired first half, the token the `xeu`
-- state fails at). That granularity is the lexer's everywhere, not this rule's.
SELECT E'\uDCA9';
