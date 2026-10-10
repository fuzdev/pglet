-- The `text`-typed operand, which is the case an unknown literal is *not*: a cast
-- gives the literal a type, and a typed `text` is `42804` even when its value is a
-- boolean spelling (`'t'::text IS TRUE` is `42804` on 18.6, measured).
-- @test
-- @expect-error: argument of IS TRUE must be type boolean, not type text
SELECT 't'::text IS TRUE;
