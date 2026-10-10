-- The accept side of the fixed-parameter gate: pglet's session zone is UTC and stays UTC, so a
-- statement that sets it to `UTC` is honored as written and `SHOW` answers what 18.6 answers
-- after it. Refusing this is the over-correction the gate invites, and it is what a harness or
-- a pool sends to pin a session. The generic `SET <name> =` spelling.
-- @test
SET TimeZone = 'utc';
SHOW TimeZone;
