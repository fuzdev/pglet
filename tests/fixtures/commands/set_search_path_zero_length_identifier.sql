-- The `search_path` spelling of the same thing, and a regression the decider once introduced
-- and then closed: the decider's "an empty element is the empty list" arm
-- could not tell a zero-length *string* (`''`, which 18.6 accepts and pglet honours)
-- from a zero-length *delimited identifier* (`""`, which 18.6's scanner refuses), so
-- `SET search_path TO ""` was answered `SET`. It is the lexer's now, so the decider
-- never sees the token at all.
-- @test
-- @expect-error: zero-length delimited identifier
SET search_path TO public, "";
