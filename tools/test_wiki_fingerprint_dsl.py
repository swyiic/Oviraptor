"""Offline semantic regressions: python3 -m unittest discover -s tools -p test_wiki_fingerprint_dsl.py."""
import json
import itertools
import re
import unittest

from wiki_fingerprint_dsl import (ConversionError, convert_query, convert_query_tree, normalize_query,
                                  query_predicates, query_predicate_tree, query_predicates_tree)


class ConversionTests(unittest.TestCase):
    def rejects(self, query, code=None):
        with self.assertRaises(ConversionError) as context:
            convert_query(query)
        if code:
            self.assertEqual(context.exception.code, code)

    def test_predicate_api_returns_parsed_inner_literals_not_regex_fragments(self):
        literal = '<a href="/product">'
        query = json.dumps({"GobyQuery": 'body=' + json.dumps(literal) + ' && title="Product"'})
        predicates = query_predicates(query)
        self.assertIn({"field": "body", "value": literal}, predicates)
        self.assertIn({"field": "title", "value": "Product"}, predicates)
        with self.assertRaises(ConversionError):
            query_predicates('body="Alpha" && (body="Beta" || body="Gamma")')

    def test_body_literal_is_escaped_and_not_case_folded(self):
        result = convert_query('body="Product.(js)[1]"')
        pattern = result['bodyRegex'][0]
        self.assertTrue(re.search(pattern, 'Product.(js)[1]'))
        self.assertFalse(re.search(pattern, 'productXjs1'))
        self.assertEqual(result['matchMode'], 'any')

    def test_html_in_literal_is_not_stripped(self):
        literal = '<meta name="generator" content="MyProduct">'
        query = 'body=' + json.dumps(literal)
        result = convert_query(query)
        self.assertTrue(re.search(result['bodyRegex'][0], literal))
        self.assertFalse(re.search(result['bodyRegex'][0], 'MyProduct'))

    def test_title_scopes_content_and_preserves_value_case(self):
        result = convert_query('title="WordPress"')
        pattern = result['bodyRegex'][0]
        self.assertTrue(re.search(pattern, '<TITLE>Welcome WordPress Dashboard</TITLE>'))
        self.assertFalse(re.search(pattern, '<title>wordpress</title>'))
        self.assertFalse(re.search(pattern, '<title>Other</title><p>WordPress</p>'))

    def test_title_does_not_guess_entity_or_markup_semantics(self):
        for value in ['A&B', 'A&lt;B', '<b>Product</b>']:
            self.rejects('title=' + json.dumps(value), 'unsupported_title_encoding')

    def test_flat_and_requires_every_condition(self):
        result = convert_query('body="Alpha" && (body="Beta" && server="Vendor")')
        self.assertEqual(result['matchMode'], 'all')
        self.assertEqual(len(result['bodyRegex']), 2)
        self.assertEqual(result['headerRegex'], {'server': 'Vendor'})

    def test_flat_or_and_same_header_alternation(self):
        result = convert_query('(server="VendorA" || server="VendorB") || body="Product"')
        self.assertEqual(result['matchMode'], 'any')
        pattern = result['headerRegex']['server']
        self.assertTrue(re.search(pattern, 'VendorA/1'))
        self.assertTrue(re.search(pattern, 'VendorB/2'))
        self.assertFalse(re.search(pattern, 'VendorC'))

    def test_mixed_boolean_is_not_flattened(self):
        for query in ['body="Alpha" && (body="Beta" || body="Gamma")',
                      'body="Alpha" || body="Beta" && body="Gamma"']:
            self.rejects(query, 'mixed_boolean')

    def test_duplicate_subexpressions_can_collapse_without_changing_logic(self):
        result = convert_query('(body="Alpha" && body="Beta") || (body="Beta" && body="Alpha")')
        self.assertEqual(result['matchMode'], 'all')
        self.assertEqual(len(result['bodyRegex']), 2)

    def test_unsupported_atom_rejects_entire_expression(self):
        for field in ['app', 'product', 'icon_hash', 'banner', 'protocol', 'port', 'country', 'url', 'cert']:
            self.rejects(f'body="Product" || {field}="Value"', 'unsupported_field')

    def test_no_guessed_negative_exact_fuzzy_or_operator_dialects(self):
        for query in ['body=="Product"', 'body!="Product"', 'body*="Prod*"',
                      '!body="Product"', 'body="Product" & title="Title"',
                      'body="Product" | title="Title"', 'body="Product" AND title="Title"']:
            self.rejects(query)

    def test_named_header_retains_prefix_position(self):
        result = convert_query('header="Server: Vendor"')
        self.assertEqual(result['headerRegex'], {'server': r'\AVendor'})
        self.assertTrue(re.search(result['headerRegex']['server'], 'Vendor/1'))
        self.assertFalse(re.search(result['headerRegex']['server'], 'Other Vendor'))
        self.rejects('header="Vendor"', 'unsupported_header_shape')

    def test_same_header_and_does_not_drop_a_condition(self):
        self.rejects('server="Alpha" && server="Beta"', 'same_header_conjunction')
        result = convert_query('server="Alpha" && server="Alpha"')
        self.assertEqual(result['headerRegex'], {'server': 'Alpha'})

    def test_sensitive_headers_are_not_exported(self):
        for name in ['Set-Cookie', 'Authorization', 'Proxy-Authorization', 'X-API-Key', 'X-CSRF-Token', 'WWW-Authenticate']:
            self.rejects('header=' + json.dumps(name + ': sample'), 'unsupported_sensitive_header')

    def test_known_complete_markdown_wrappers(self):
        for query in ['> title="Product"', '+ fofa`body="Product"`',
                      'fofa-query: \'body="Product"\'', 'FOFA：`body="Product"`',
                      '```fofa\nbody="Product"\n```']:
            self.assertEqual(len(convert_query(query)['bodyRegex']), 1)

    def test_json_wrapper_decodes_exactly_one_literal_layer(self):
        inner = 'body=' + json.dumps('<a href="/product">')
        query = json.dumps({'GobyQuery': inner})
        self.assertEqual(normalize_query(query), inner)
        pattern = convert_query(query)['bodyRegex'][0]
        self.assertTrue(re.search(pattern, '<a href="/product">'))
        self.rejects(r'body=\"Product\"')
        self.rejects(json.dumps({'GobyQuery': inner, 'ScanSteps': []}), 'unsupported_json_wrapper')

    def test_duplicate_json_query_keys_are_ambiguous_and_rejected(self):
        self.rejects('{"GobyQuery":"body=\\\"Alpha\\\"","GobyQuery":"body=\\\"Beta\\\""}', 'duplicate_json_key')

    def test_partial_json_or_prose_is_not_substring_extracted(self):
        for query in ['例如: body="Product"', '语法：body="Product"',
                      'body="Product" 更多说明', '"GobyQuery":"body=\\"Product\\"",',
                      '<font>body="Product"</font>', '`body="Product"']:
            self.rejects(query)

    def test_unknown_backslash_escape_is_not_discarded(self):
        result = convert_query(r'body="C:\Product"')
        self.assertTrue(re.search(result['bodyRegex'][0], r'C:\Product'))
        self.assertFalse(re.search(result['bodyRegex'][0], 'C:Product'))

    def test_aliases_empty_literals_and_complexity_boundaries(self):
        self.assertEqual(convert_query('web.body="Product"'), convert_query('body="Product"'))
        self.assertEqual(convert_query('web.title="Product"'), convert_query('title="Product"'))
        for query in ['body=""', 'body=" "', 'body="..."', 'body=Product', '(body="Product"', '']:
            self.rejects(query)
        self.rejects('(' * 35 + 'body="Product"' + ')' * 35, 'query_too_complex')
        self.rejects('body="' + 'a' * 33000 + '"', 'query_too_long')


class BooleanTreeTests(unittest.TestCase):
    @staticmethod
    def evaluate(node,body='',headers=None):
        headers=headers or {}
        if node['type']=='all':return all(BooleanTreeTests.evaluate(child,body,headers) for child in node['children'])
        if node['type']=='any':return any(BooleanTreeTests.evaluate(child,body,headers) for child in node['children'])
        value=body if node['type']=='body' else headers.get(node['name'],'')
        return bool(re.search(node['pattern'],value))

    def test_old_any_serialization_and_ids_are_not_changed(self):
        for query in ['body="Product"','server="VendorA" || server="VendorB"','(title="WordPress") || body="wp-content"']:
            self.assertEqual(convert_query_tree(query),convert_query(query))

    def test_nested_truth_tables_preserve_precedence_and_parentheses(self):
        cases=[('body="Alpha" && (body="Beta" || body="Gamma")',lambda a,b,c:a and (b or c)),
               ('body="Alpha" || body="Beta" && body="Gamma"',lambda a,b,c:a or (b and c)),
               ('(body="Alpha" || body="Beta") && body="Gamma"',lambda a,b,c:(a or b) and c)]
        for query,expected in cases:
            node=convert_query_tree(query)['condition']
            for flags in itertools.product([False,True],repeat=3):
                body=' '.join(word for word,enabled in zip(['Alpha','Beta','Gamma'],flags) if enabled)
                self.assertEqual(self.evaluate(node,body),expected(*flags),(query,flags))

    def test_flat_all_and_repeated_header_conjunction_use_nodes(self):
        node=convert_query_tree('server="Alpha" && server="Beta"')['condition']
        self.assertEqual(node['type'],'all')
        self.assertEqual(len(node['children']),2)
        self.assertTrue(self.evaluate(node,headers={'server':'Alpha Beta'}))
        self.assertFalse(self.evaluate(node,headers={'server':'Alpha'}))
        self.assertFalse(self.evaluate(node,headers={'server':'Beta'}))

    def test_no_unsupported_or_sensitive_clause_is_hidden_by_strong_brand(self):
        for query in ['body="Brand" && (body="login" || app="Known")',
                      'body="Brand" && header="Set-Cookie: session=example"',
                      'body="Brand" && body!="missing"']:
            with self.assertRaises(ConversionError):convert_query_tree(query)

    def test_literal_tree_keeps_raw_values_for_privacy_checks(self):
        query='body="Brand" && (body="Login" || title="Welcome")'
        tree=query_predicate_tree(query)
        self.assertEqual(tree['type'],'all')
        self.assertEqual(len(query_predicates_tree(query)),3)
        self.assertIn({'field':'body','value':'Login'},query_predicates_tree(query))

    def test_boolean_order_and_duplicate_nodes_are_canonical(self):
        left=convert_query_tree('(body="Alpha" && body="Beta") || body="Gamma"')
        right=convert_query_tree('body="Gamma" || (body="Beta" && body="Alpha" && body="Alpha")')
        self.assertEqual(left,right)


if __name__ == '__main__':
    unittest.main()
