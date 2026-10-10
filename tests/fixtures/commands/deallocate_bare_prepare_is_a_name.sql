-- The `PREPARE` noise word is consumed only when a target follows it. `PREPARE` is
-- UNRESERVED, so `ColId` admits it and this names the statement `prepare` — `26000`,
-- not the `42601` an implementation that always ate the noise word would answer.
-- Measured on 18.6 with a real `PREPARE prepare AS SELECT 2`: it drops that one.
-- @test
-- @expect-error: prepared statement "prepare" does not exist
DEALLOCATE prepare;
