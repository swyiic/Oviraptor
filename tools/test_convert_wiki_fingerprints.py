"""Dataflow regression checks using only synthetic records; no source DB/network."""
import json
import itertools
import unittest
from convert_wiki_fingerprints import (convert_records, convert_boolean_records, evaluate_condition,
                                       expression_strength, canonical_fields, minimal_positive_dnf, existing_rule_coverage)
from wiki_fingerprint_dsl import query_predicate_tree


def source(identifier, product, query, kind='goby'):
    return {'record_id':identifier,'source_kind':kind,'source_path':'/synthetic/input.json','source_pointer':'/0/FofaQuery','source_sha256':'0'*64,'products':[product],'query':query,'eligible':True}


class PipelineTests(unittest.TestCase):
    def test_current_ANY_format_has_no_AND_downgrade(self):
        rules,rows=convert_records([source('a','Example','body="ExampleA" && body="ExampleB"'),source('b','Example','body="ExampleA" || body="ExampleB"')],[])
        self.assertEqual(len(rules),1)
        self.assertEqual(rows[0]['reason'],'current_loader_has_no_AND_support')
        self.assertNotIn('matchMode',rules[0])
        self.assertEqual(len(rules[0]['bodyRegex']),2)

    def test_duplicate_sources_do_not_duplicate_rules_or_overwrite_builtin(self):
        builtin=[{'id':'existing','name':'Existing','bodyRegex':['old-pattern']}]
        original=json.dumps(builtin,sort_keys=True)
        rules,rows=convert_records([source('a','New','body="NewAppMarker"'),source('b','New','body="NewAppMarker"'),source('c','Existing','body="NewExistingMarker"')],builtin)
        self.assertEqual(len(rules),1)
        self.assertEqual([r['status'] for r in rows],['converted','duplicate','protected_existing'])
        self.assertEqual(json.dumps(builtin,sort_keys=True),original)

    def test_derived_article_product_requires_explicit_association(self):
        records=[source('wrong','VendorA','body="OtherProductMarker"','feature_dsl'),source('right','VendorA','title="VendorA Console"','feature_dsl')]
        rules,rows=convert_records(records,[])
        self.assertEqual(len(rules),1)
        self.assertEqual(rows[0]['reason'],'unconfirmed_product_association')
        self.assertEqual(rules[0]['name'],'VendorA')

    def test_generic_OR_branch_and_historical_secret_are_rejected_whole(self):
        records=[source('shared','App','body="UniqueBrand" || body="/static/thirdparty/jquery/"'),source('401','Router','title="401 Authorization Is Required" || server="simple httpd1.0"'),source('secret','App','body=\'"accessKeyId":"AK123456789012345678"\'')]
        rules,rows=convert_records(records,[])
        self.assertEqual(rules,[])
        self.assertEqual(rows[0]['reason'],'shared_dependency_not_product_identity')
        self.assertEqual(rows[1]['reason'],'generic_or_non_product_specific_marker')
        self.assertEqual(rows[2]['reason'],'historical_credential_or_session_literal')

    def test_common_server_cannot_be_renamed_as_unrelated_app(self):
        rules,rows=convert_records([source('wrong','Hasura GraphQL','server="Warp"'),source('right','Warp','server="Warp"')],[])
        self.assertEqual(len(rules),1)
        self.assertEqual(rules[0]['name'],'Warp')
        self.assertEqual(rows[0]['reason'],'server_component_not_application_identity')

    def test_generic_cache_headers_are_not_application_fingerprints(self):
        rules,rows=convert_records([source('cache','ShippingApp','header="Cache-Control: no-store, no-cache"'),source('etag','CameraApp','header=\'ETag: "1d2\' || body="UniqueBrand"')],[])
        self.assertEqual(rules,[])
        self.assertTrue(all(r['reason']=='generic_response_header_not_product_identity' for r in rows))

    def test_query_text_is_not_a_product_name(self):
        rules,rows=convert_records([source('wrong','title="brightsign"','title="brightsign"')],[])
        self.assertEqual(rules,[])
        self.assertEqual(rows[0]['reason'],'missing_product_context')

    def test_conflicting_product_labels_are_not_merged_by_identical_weak_signal(self):
        rules,rows=convert_records([source('a','BrandA','body="CommonConsoleMarker"'),source('b','BrandB','body="CommonConsoleMarker"')],[])
        self.assertEqual(rules,[])
        self.assertTrue(all(r['reason']=='ambiguous_products_for_identical_signal' for r in rows))


class BooleanPipelineTests(unittest.TestCase):
    def test_strong_brand_can_require_weak_login_without_dropping_it(self):
        rules,rows=convert_boolean_records([source('a','Example','body="ExampleBrand" && title="Login"')],[])
        self.assertEqual(len(rules),1)
        node=rules[0]['condition']
        self.assertFalse(evaluate_condition(node,'ExampleBrand'))
        self.assertFalse(evaluate_condition(node,'<title>Login</title>'))
        self.assertTrue(evaluate_condition(node,'ExampleBrand<title>Login</title>'))
        self.assertIn('generic_or_non_product_specific_marker',rows[0]['supportive_conditions'])

    def test_every_OR_alternative_must_have_specific_evidence(self):
        records=[source('unsafe','Example','(body="ExampleBrand" && body="login") || title="Login"'),
                 source('safe','Example','body="ExampleBrand" && (body="login" || title="Login")')]
        rules,rows=convert_boolean_records(records,[])
        self.assertEqual(len(rules),1)
        self.assertEqual(rows[0]['reason'],'boolean_branch_without_specific_evidence')
        self.assertEqual(rows[1]['status'],'converted')

    def test_shared_JS_and_generic_cache_are_supportive_only_inside_AND(self):
        records=[source('safe','Example','body="ExampleBrand" && (body="/jquery/jquery.js" || header="Cache-Control: no-cache")'),
                 source('unsafe','Example','body="/jquery/jquery.js" && header="Cache-Control: no-cache"')]
        rules,rows=convert_boolean_records(records,[])
        self.assertEqual(len(rules),1)
        self.assertEqual(rows[1]['reason'],'boolean_branch_without_specific_evidence')
        self.assertFalse(evaluate_condition(rules[0]['condition'],'/jquery/jquery.js',{'cache-control':'no-cache'}))

    def test_privacy_failure_rejects_even_under_mandatory_brand(self):
        query='body="ExampleBrand" && body=\'"accessKeyId":"AK123456789012345678"\''
        rules,rows=convert_boolean_records([source('secret','Example',query)],[])
        self.assertEqual(rules,[])
        self.assertEqual(rows[0]['reason'],'historical_credential_or_session_literal')

    def test_incremental_mode_preserves_legacy_catalog_and_builtin_rules(self):
        builtin=[{'id':'existing','name':'Existing','bodyRegex':['old-pattern']}]
        original=json.dumps(builtin,sort_keys=True)
        rules,rows=convert_boolean_records([source('flat','New','body="NewMarker"'),
                                          source('existing','Existing','body="ExistingBrand" && body="login"')],builtin)
        self.assertEqual(rules,[])
        self.assertEqual(rows[0]['status'],'not_incremental')
        self.assertEqual(rows[1]['status'],'protected_existing')
        self.assertEqual(json.dumps(builtin,sort_keys=True),original)

    def test_condition_order_and_source_duplicates_produce_one_stable_ID(self):
        records=[source('a','Example','body="ExampleBrand" && body="LoginForm"'),
                 source('b','Example','body="LoginForm" && body="ExampleBrand"')]
        rules,rows=convert_boolean_records(records,[])
        self.assertEqual(len(rules),1)
        self.assertTrue(rules[0]['id'].startswith('wikibool-'))
        self.assertEqual([row['status'] for row in rows],['converted','duplicate'])
        repeated,_=convert_boolean_records(list(reversed(records)),[])
        self.assertEqual(rules,repeated)

    def test_known_wrong_product_pairing_and_unconfirmed_article_labels_stay_out(self):
        records=[source('wrong','KubeOperator','title="KubePi" && body="KubeOperator"'),
                 source('article','OtherVendor Extra','body="OtherVendorMarker" && body="login"','feature_dsl')]
        rules,rows=convert_boolean_records(records,[])
        self.assertEqual(rules,[])
        self.assertEqual(rows[0]['reason'],'reviewed_source_product_mismatch')
        self.assertEqual(rows[1]['reason'],'unconfirmed_product_association')

    def test_generic_combinations_do_not_become_specific_merely_by_AND(self):
        records=[source('generic','popub','title="档案局" && body="/js/MSClass.js"'),
                 source('images','云视讯','body="userName.png" && body="verifyCode.png"'),
                 source('good','Opencast-Matterhorn','body="opencast" && body="j_spring_security_check"')]
        rules,rows=convert_boolean_records(records,[])
        self.assertEqual(len(rules),1)
        self.assertEqual(rules[0]['name'],'Opencast-Matterhorn')
        self.assertEqual(rows[0]['reason'],'boolean_branch_without_specific_evidence')
        self.assertEqual(rows[1]['reason'],'boolean_branch_without_specific_evidence')

    def test_vendor_license_or_generic_chinese_category_is_not_exact_product(self):
        records=[source('vendor','Apache Kylin','body="Apache License" && body="LoginForm"'),
                 source('category','panalog日志系统','(body="日志系统" && body="codeno") || title="panalog"')]
        rules,rows=convert_boolean_records(records,[])
        self.assertEqual(rules,[])
        self.assertTrue(all(row['reason']=='boolean_branch_without_specific_evidence' for row in rows))

    def test_branch_specificity_agrees_with_exhaustive_weak_only_witnesses(self):
        queries=[
            ('body="ExampleA" && (body="login" || body="password")',lambda a,b,c,d:a and (b or d)),
            ('body="ExampleA" || (body="login" && body="password")',lambda a,b,c,d:a or (b and d)),
            ('(body="ExampleA" || body="login") && (body="ExampleC" || body="password")',lambda a,b,c,d:(a or b) and (c or d)),
            ('(body="ExampleA" && body="login") || (body="ExampleC" && body="password")',lambda a,b,c,d:(a and b) or (c and d)),
        ]
        for query,truth in queries:
            strong,_=expression_strength(query_predicate_tree(query),['Example'])
            weak_only_witness=False
            for flags in itertools.product([False,True],repeat=4):
                if truth(*flags) and not flags[0] and not flags[2]:weak_only_witness=True
            self.assertEqual(strong,not weak_only_witness,query)

    def test_reviewed_generic_pairs_do_not_claim_a_specific_vendor(self):
        records=[source('reporter','TopSec Reporter','title="Login @ Reporter" && body="height:125px;" && body="width:310px;"'),
                 source('ricon','Ricon Industrial Cellular Router','server="WEB-ROUTER" && body="Industrial Cellular"'),
                 source('ac','深圳维盟科技股份有限公司AC集中管理平台','title="AC集中管理平台" && body="login_25.jpg"')]
        rules,rows=convert_boolean_records(records,[])
        self.assertEqual(rules,[])
        self.assertTrue(all(row['reason'].startswith('reviewed_generic_') for row in rows))

    def test_family_only_marker_does_not_claim_a_specific_IVMS_model(self):
        rules,rows=convert_boolean_records([source('family','视频联网监控系统IVMS-8100','body="/ivms-ui/" && body="/logincheck"')],[])
        self.assertEqual(rules,[])
        self.assertEqual(rows[0]['reason'],'unconfirmed_model_family_marker')

    def test_vendor_or_family_title_does_not_confirm_specific_models(self):
        records=[source('iball','iBall ADSL2+ Home Router WRA150N','title="ADSL2+ Router" && title="iBall"'),
                 source('zyxel','ZyXEL Keenetic Lite III','title="Detailed summary" && title="ZyXEL Keenetic"'),
                 source('wap','ZYXEL WAP系列无线路由器','body="ZYXEL Communications Corp" && body="login.cgi"')]
        rules,rows=convert_boolean_records(records,[])
        self.assertEqual(rules,[])
        self.assertTrue(all(row['reason']=='unconfirmed_model_family_marker' for row in rows))

    def test_reviewed_ambiguous_source_slashes_are_not_silently_repaired(self):
        literal='href="\\/kylin\\/"'
        query='body='+json.dumps(literal)+' && body="to you under the Apache License"'
        rules,rows=convert_boolean_records([source('escape','Apache-kylin',query)],[])
        self.assertEqual(rules,[])
        self.assertEqual(rows[0]['reason'],'ambiguous_source_escape')

    def test_same_product_old_Any_atom_covers_new_AND_without_rewriting_it(self):
        old=[{'id':'old-example','name':'Example','bodyRegex':['ExampleBrand']}]
        original=json.dumps(old,sort_keys=True)
        rules,rows=convert_boolean_records([source('new','Example','body="ExampleBrand" && body="RequiredExtra"')],[],old)
        self.assertEqual(rules,[])
        self.assertEqual(rows[0]['reason'],'existing_rule_covers_all_branches')
        self.assertEqual(rows[0]['covered_by'],['old-example'])
        self.assertEqual(json.dumps(old,sort_keys=True),original)

    def test_uncovered_OR_branch_is_kept_even_when_one_branch_is_old(self):
        old=[{'id':'old-example','name':'Example','bodyRegex':['ExampleBrand']}]
        query='(body="ExampleBrand" && body="RequiredExtra") || (body="ExampleOther" && body="LoginForm")'
        rules,rows=convert_boolean_records([source('new','Example',query)],[],old)
        self.assertEqual(len(rules),1)
        self.assertEqual(rows[0]['status'],'converted')

    def test_coverage_never_guesses_product_alias_field_or_regex_implication(self):
        current=[source('new','Example','body="ExampleBrandX" && body="RequiredExtra"')]
        cases=[{'id':'other-name','name':'Other App','bodyRegex':['ExampleBrandX']},
               {'id':'other-field','name':'Example','headerRegex':{'server':'ExampleBrandX'}},
               {'id':'substring','name':'Example','bodyRegex':['ExampleBrand']}]
        for old in cases:
            rules,_=convert_boolean_records(current,[],[old])
            self.assertEqual(len(rules),1,old)

    def test_DNF_dedup_bound_is_unknown_and_does_not_discard_data(self):
        brand={'type':'body','pattern':'ExampleBrand'}
        condition={'type':'all','children':[brand]+[
            {'type':'any','children':[{'type':'body','pattern':f'Example{i}A'},
                                      {'type':'body','pattern':f'Example{i}B'}]} for i in range(11)]}
        self.assertIsNone(minimal_positive_dnf(condition))
        self.assertEqual(existing_rule_coverage('Example',condition,[{'id':'old','name':'Example','bodyRegex':['ExampleBrand']}]),[])
        small={'type':'any','children':[brand,{'type':'all','children':[brand,{'type':'body','pattern':'Other'}]}]}
        self.assertEqual(minimal_positive_dnf(small),[frozenset({('body','ExampleBrand')})])

    def test_same_header_AND_remains_two_constraints(self):
        rules,_=convert_boolean_records([source('a','Example','server="ExampleA" && server="ExampleB"')],[])
        self.assertEqual(len(rules),1)
        self.assertFalse(evaluate_condition(rules[0]['condition'],headers={'server':'ExampleA'}))
        self.assertTrue(evaluate_condition(rules[0]['condition'],headers={'server':'ExampleA ExampleB'}))


if __name__=='__main__':unittest.main()
