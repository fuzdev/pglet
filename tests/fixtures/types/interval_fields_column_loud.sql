-- A fields-qualified interval column (`INTERVAL DAY TO SECOND`) reinterprets and
-- truncates fields; pglet rejects it rather than silently mis-truncate. Postgres
-- accepts it, so this is a deliberate pglet-stricter case (any pglet error passes).
-- @test
-- @expect-error: fields-qualified interval column unsupported
CREATE TABLE iv_fields (x INTERVAL DAY TO SECOND);
