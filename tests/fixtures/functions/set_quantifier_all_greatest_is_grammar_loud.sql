-- The `ALL` polarity of the grammar-name rule, and the reason the quantifier has to be
-- carried as *which* keyword rather than dropped: `ALL` is a no-op on any call that
-- resolves, so a parser that simply ate it would answer `2` here where 18.6 refuses the
-- statement outright. 42601 on both.
-- @test
-- @expect-error: syntax error at or near "ALL"
SELECT greatest(ALL 1, 2);
