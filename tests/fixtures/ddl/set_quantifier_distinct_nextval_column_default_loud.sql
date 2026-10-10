-- The refusal polarity at the classifier route. `nextval` is the only arm of
-- `parser::ddl`'s `try_function_default` whose call takes an argument, so it is the only
-- member: the six niladic arms sit behind an `expect(RParen)` that reproduces 18.6's own
-- grammar bound (`DEFAULT now(ALL)` is `42601` on both). This spelling was `42601` here
-- while its parenthesized twin — the same statement, one pair of parens apart — was
-- already the `42809` below. The sequence is seeded so the verdict is the only thing
-- this fixture can be answering: over a name no sequence holds, 18.6 answers the same
-- `42809` ahead of its own `42P01`, but that is the neighbouring ordering's subject.
-- @setup
CREATE SEQUENCE sq_ndd_seq;
-- @test
-- @expect-error: DISTINCT specified, but nextval is not an aggregate function
CREATE TABLE sq_ndd (a BIGINT DEFAULT nextval(DISTINCT 'sq_ndd_seq'));
