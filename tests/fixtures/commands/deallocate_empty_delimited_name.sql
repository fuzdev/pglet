-- `""` is refused by the *scanner*, so this is `42601` and not the `42602`
-- (`invalid_name`) that pglet's name-literal reader raises for the analogous
-- `nextval('')`. Measured on 18.6, since the two are easy to conflate.
-- @test
-- @expect-error: zero-length delimited identifier
DEALLOCATE "";
