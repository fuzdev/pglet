-- The other overflow class, and the one whose SQLSTATE **does** agree: a *field*
-- one past `i32` under a `DAY` qualifier is `22015` on both engines. Paired with
-- `interval_qualifier_year_overflow_is_loud.sql` (`22008` there, the result rather
-- than the field) so a single code standing in for both cannot pass.
-- @test
-- @expect-error: interval field value out of range
SELECT INTERVAL '2147483648' DAY;
