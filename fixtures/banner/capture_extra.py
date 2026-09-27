import sys, json, http.cookiejar, urllib.request, urllib.parse
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from capture import Recorder
cfg=json.loads((Path(__file__).resolve().parents[1]/'sources.json').read_text())
r=Recorder()
for label, term, query in [('empty',cfg['term'],'kampuszzzznonexistent'),('second-term','202536',None)]:
    jar=http.cookiejar.CookieJar()
    opener=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
    r.capture('banner',label+'-session',cfg['banner']+'/term/search?mode=search',opener,{'term':term})
    params={'txt_term':term,'txt_subject':cfg['subject'],'pageOffset':0,'pageMaxSize':10,'sortColumn':'subjectDescription','sortDirection':'asc'}
    if query: params['txt_keywordlike']=query
    body,_=r.capture('banner',label+'-search',cfg['banner']+'/searchResults/searchResults?'+urllib.parse.urlencode(params),opener)
    if body:
        d=json.loads(body)
        print(label, d.get('success'), d.get('totalCount'), len(d['data']) if isinstance(d.get('data'),list) else 'invalid')
    jar.clear()
