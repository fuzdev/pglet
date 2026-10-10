-- The second persisted-expression site. A stored CHECK carrying a call 18.6 refuses
-- outright is the worse over-acceptance of the four: the table is created and every
-- write is then judged by an expression the source never wrote. 42809 on both.
-- @test
-- @expect-error: DISTINCT specified, but upper is not an aggregate function
CREATE TABLE sq_dck (c TEXT, CHECK (upper(DISTINCT c) = 'A'));
