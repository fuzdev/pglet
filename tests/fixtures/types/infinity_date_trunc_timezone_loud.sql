-- A zone field is refused for an infinite timestamp as for a finite one; the sentence
-- names the type differently here, so only the code is compared.
-- @expect-error: unit "timezone" not supported for type timestamp
-- @test
SELECT date_trunc('timezone', timestamp 'infinity');
