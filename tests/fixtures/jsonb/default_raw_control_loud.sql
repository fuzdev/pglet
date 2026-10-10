-- The reader at a DDL position, where what the divergence costs is the *catalog*: this
-- statement used to be accepted, and the resulting database dumped at exit `0` into SQL
-- whose first statement 18.6 refuses, so the restore created nothing.
-- @expect-error: a raw tab in a stored default
-- @expect-message
-- @test
CREATE TABLE dft (id int, v jsonb DEFAULT (E'{"a\tb": 1}')::jsonb);
