-- The `ALTER TABLE … ADD CONSTRAINT … REFERENCES` route resolves its parent through a
-- second lookup, which needed the same kind check as the CREATE TABLE one.
-- @setup
CREATE SEQUENCE arefs_seq;
CREATE TABLE arefs_child (b INT);
-- @test
ALTER TABLE arefs_child ADD CONSTRAINT arefs_fk FOREIGN KEY (b) REFERENCES arefs_seq(a);
-- @expect-error: a sequence cannot be referenced (42809)
-- @no-error-fields: 18.6 answers this route with `referenced relation "x" is not a table` and no DETAIL; pglet reaches it through the `cannot open relation` wording, which carries one
