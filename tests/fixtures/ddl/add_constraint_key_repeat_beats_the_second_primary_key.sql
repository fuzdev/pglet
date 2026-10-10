-- And it outranks `42P16`, which on the `ALTER` routes sits inside `DefineIndex` —
-- below parse analysis, where the repeat lives. The `CREATE TABLE` spelling puts the
-- two the other way for a constraint's *own* multiplicity
-- (`create_table_key_repeat_beats_a_second_primary_key.sql`).
-- @setup
CREATE TABLE ackm (a INT NOT NULL, b INT);
ALTER TABLE ackm ADD CONSTRAINT ackm_pk0 PRIMARY KEY (a);
-- @expect-error: the repeated key column outranks the second primary key
-- @expect-message
-- @no-profile-lock: the @setup builds the table's enforced PRIMARY KEY index so the @test has a second one to collide with; the refusal is a parse-analysis verdict raised before any plan
-- @test
ALTER TABLE ackm ADD CONSTRAINT ackm_pk1 PRIMARY KEY (b, b);
