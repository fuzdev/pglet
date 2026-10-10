-- `transaction_timestamp`, the fourth `now`-family spelling.
-- @test
-- @expect-error: function transaction_timestamp(integer) does not exist
SELECT transaction_timestamp(1);
