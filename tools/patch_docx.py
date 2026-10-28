"""给 ui/vendor/docx-preview.min.js 打 Peek 补丁：识别按字符数写的缩进。升级该库后重跑。"""
import os, sys
P = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'ui', 'vendor', 'docx-preview.min.js')
s = open(P, encoding='utf8').read()
if 'Peek 补丁：*Chars' in s:
    sys.exit('已经打过补丁')
old = 'parseIndentation(e,t){var r=w.lengthAttr(e,"firstLine"),a=w.lengthAttr(e,"hanging"),s=w.lengthAttr(e,"left"),n=w.lengthAttr(e,"start"),l=w.lengthAttr(e,"right"),o=w.lengthAttr(e,"end");'
assert s.count(old) == 1, s.count(old)
# 按字符数写的缩进（firstLineChars/hangingChars/leftChars/rightChars，中文 Word/WPS 最常见）：
# 非零时优先于 twips 值，单位是 1/100 字宽，用 em 近似；原库不认这些属性，编号段落的负首行缩进就伸出了表格边框
new = old + ('/* Peek 补丁：*Chars 缩进 */var C=function(k){var v=w.attr(e,k);return v&&0!=parseInt(v)?parseInt(v)/100+"em":null};'
             'r=C("firstLineChars")||r,a=C("hangingChars")||a,s=C("leftChars")||C("startChars")||s,l=C("rightChars")||C("endChars")||l;')
s = s.replace(old, new)
open(P, 'w', encoding='utf8', newline='').write(s)
print('patched')
