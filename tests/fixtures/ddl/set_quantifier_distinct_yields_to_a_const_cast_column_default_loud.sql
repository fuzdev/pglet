-- The same rule at the persisted-expression site, through the general expression parser
-- (`upper` is not the classifier's `nextval`, so `parse_column_default` routes it to
-- `build_call` like any other call). Its parenthesized twin
-- `DEFAULT (upper(DISTINCT 'a'::int))` measures identically on both engines, which is what
-- keeps `parse_column_default`'s one-stored-fact invariant true of the refusal as well as
-- of the accept. 22P02 on both.
-- @test
-- @expect-error: invalid input syntax for type integer: "a"
CREATE TABLE cc_dflt (c TEXT DEFAULT upper(DISTINCT 'a'::int));
