"""给 ui/vendor/docx-preview.min.js 打 Peek 补丁。升级该库后重跑；已打过的补丁会跳过。"""
import os, sys
P = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'ui', 'vendor', 'docx-preview.min.js')
s = open(P, encoding='utf8').read()
PATCHES = []

# 按字符数写的缩进（firstLineChars/hangingChars/leftChars/rightChars，中文 Word/WPS 最常见）：
# 非零时优先于 twips 值，单位是 1/100 字宽，用 em 近似；原库不认这些属性，编号段落的负首行缩进就伸出了表格边框
old = 'parseIndentation(e,t){var r=w.lengthAttr(e,"firstLine"),a=w.lengthAttr(e,"hanging"),s=w.lengthAttr(e,"left"),n=w.lengthAttr(e,"start"),l=w.lengthAttr(e,"right"),o=w.lengthAttr(e,"end");'
PATCHES.append(('Peek 补丁：*Chars', old, old + (
    '/* Peek 补丁：*Chars 缩进 */var C=function(k){var v=w.attr(e,k);return v&&0!=parseInt(v)?parseInt(v)/100+"em":null};'
    'r=C("firstLineChars")||r,a=C("hangingChars")||a,s=C("leftChars")||C("startChars")||s,l=C("rightChars")||C("endChars")||l;')))

# 分节符：原库只在显式分页符/lastRenderedPageBreak/纸张变化处分页。Word 保存时会写 lastRenderedPageBreak，
# WPS 不写，于是 WPS 文档的“下一页”分节全被并成一页。这里按下一节的 type（缺省 nextPage）在节末分页。
PATCHES.append(('Peek 补丁：分节分页',
    '(s||-1!=n)&&(r.sectProps=s,r.pageBreak=-1!=n,',
    # Word 会在新一节/新一页的第一段开头再写一个 lastRenderedPageBreak，刚分过页就不再分，免得多出空白页
    '(-1!=n&&0==l&&e.children.slice(0,n).every(c=>!c.children?.length)&&1==r.elements.length&&a.length>1&&(n=-1)),(s||-1!=n||(F=-1==n&&!!e.children?.some(c=>c.children?.some(d=>d.pageFill))))&&(r.sectProps=s,r.sectEnd=!!s,r.pageBreak=-1!=n||F,'))
PATCHES.append(('Peek 补丁：分节分页', 'var s=e.sectionProps,n=-1,l=-1;', 'var s=e.sectionProps,n=-1,l=-1,F=!1;'))
PATCHES.append(('Peek 补丁：分节分页',
    'for(let s of e)r.push(s),(this.options.ignoreLastRenderedPageBreak||s.pageBreak||this.isPageBreakSection(t,s.sectProps))&&a.push(r=[]),t=s.sectProps;',
    '/* Peek 补丁：分节分页 */for(let i=0;i<e.length;i++){let s=e[i];r.push(s),(this.options.ignoreLastRenderedPageBreak||s.pageBreak||this.isPageBreakSection(t,s.sectProps)'
    '||s.sectEnd&&e[i+1]&&"continuous"!=e[i+1].sectProps?.type)&&a.push(r=[]),t=s.sectProps}'))

# 四周型/紧密型环绕的锚定图：原库不认偏移，统统按行内块放在段首（WPS 封面整页图会挤在正文里、页眉横幅错位）。
# 改成左浮动 + 相对栏/段落的偏移；高到一页放不下正文的（整页封面）不占高度，并在所在段落后分页。
PATCHES.append(('Peek 补丁：环绕锚定图', 'case"wrapNone":a="wrapNone";break;',
    'case"wrapNone":a="wrapNone";break;case"wrapSquare":case"wrapTight":case"wrapThrough":a="wrap";break;'))
PATCHES.append(('Peek 补丁：环绕锚定图',
    '!r||"left"!=n.align&&"right"!=n.align||(t.cssStyle.float=n.align),t}',
    # n.align 缺省就是 "left"，所以先判环绕再判对齐，否则走不到偏移分支
    'r&&"wrap"==a&&!/^(right|center)$/.test(n.align)?(/* Peek 补丁：环绕锚定图 */t.cssStyle.float="left",'
    '/^(column|margin|character|leftMargin)$/.test(n.relative)&&n.offset&&(t.cssStyle["margin-left"]=n.offset),'
    '/^(paragraph|line)$/.test(l.relative)&&l.offset&&(t.cssStyle["margin-top"]=l.offset),'
    't.cssStyle["margin-right"]=w.lengthAttr(e,"distR",p)||"0",'
    'parseFloat(t.cssStyle.height)>=640&&(t.pageFill=!0,t.cssStyle["margin-bottom"]="-"+t.cssStyle.height)):r&&("left"==n.align||"right"==n.align)&&(t.cssStyle.float=n.align),t}'))

# 没写 headerReference/footerReference 的节按规范沿用上一节的页眉页脚，原库直接不画
PATCHES.append(('Peek 补丁：页眉沿用',
    'let o=a[r][0].sectProps;const i=this.createPageElement(',
    'let o=a[r][0].sectProps;/* Peek 补丁：页眉沿用（首页/偶数页/默认三种各自沿用） */var M=(p,c)=>[...(p||[]).filter(x=>!(c||[]).some(y=>y.type==x.type)),...(c||[])];'
    'o.headerRefs=M(this._ph,o.headerRefs),o.footerRefs=M(this._pf,o.footerRefs),this._ph=o.headerRefs,this._pf=o.footerRefs;const i=this.createPageElement('))

done = 0
for mark, old, new in PATCHES:
    if new in s:
        continue
    assert s.count(old) == 1, (mark, s.count(old))
    s = s.replace(old, new)
    done += 1
open(P, 'w', encoding='utf8', newline='').write(s)
print('patched', done)
