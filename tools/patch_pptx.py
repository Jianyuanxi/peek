"""给 ui/vendor/pptx-preview.umd.js 打 Peek 补丁（第 1–12 项）。
基准是仓库里已带 rot/flip 补丁的版本；升级该库时先手工补回 rot/flip，再跑本脚本。每项替换都核对命中次数，对不上就报错退出。"""
import os, sys
P = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'ui', 'vendor', 'pptx-preview.umd.js')
s = open(P, encoding='utf8').read()
if 'Peek 补丁：跳过不存在的部件' in s:
    sys.exit('已经打过补丁；要重打先 git checkout 该文件')


def rep(old, new, count=1):
    global s
    n = s.count(old)
    if n != count:
        sys.exit(f'expected {count} got {n}: {old[:80]}')
    s = s.replace(old, new)


# 1. [Content_Types].xml 里登记了但包里不存在的部件（如多余的 slideMaster2..19）直接跳过；原库读到就抛错，整份演示文稿一页都不出
rep('n=t.Types.Override,',
    'n=[].concat(t.Types.Override||[]).filter((function(t){return!!this._zipContents.files[t.attrs.PartName.substr(1)]}).bind(this))/* Peek 补丁：跳过不存在的部件 */,')

# 2. 单个版式/幻灯片解析出错只影响它自己；原库所有页共用一个 try，一页出错后面全部丢掉
rep('[4,(g=new Rr(m,_,this)).load()]',
    '[4,(g=new Rr(m,_,this)).load().catch((function(e){console.warn("pptx layout",m,e)}))/* Peek 补丁 */]')
rep('[4,(b=new Pr(m,_,this)).load()]',
    '[4,(b=new Pr(m,_,this)).load().catch((function(e){console.warn("pptx slide",m,e)}))/* Peek 补丁 */]')

# 3. 图片引用了不存在的关系 ID（链接图片、损坏的引用）就跳过这张图，不让整页报错
rep('p=d[I],f=p["p:blipFill"]["a:blip"].attrs["r:embed"],g=r.rels[f].target,y=new Mr(g,p,n,r,i),t.push(y);',
    'p=d[I],f=Yn.get(p,["p:blipFill","a:blip","attrs","r:embed"]),r.rels[f]&&(g=r.rels[f].target,y=new Mr(g,p,n,r,i),t.push(y))/* Peek 补丁：缺图跳过 */;')

# 4. 幻灯片按 presentation.xml 的 sldIdLst 排序（原库按文件名 slideN 的数字排，和 PowerPoint/WPS 的页序不一致），不在列表里的孤立页不显示
rep('case 4:return r.sent(),[4,this._loadContentTypes()];case 5:return r.sent(),[2]',
    'case 4:return r.sent(),[4,this._loadContentTypes()];case 5:return r.sent(),[4,this.__peekOrder()];case 6:return r.sent(),[2]')
ORDER = ('t.prototype.__peekOrder=function(){/* Peek 补丁：按 sldIdLst 排页 */var s=this,f=s._zipContents.files,a=f["ppt/presentation.xml"],b=f["ppt/_rels/presentation.xml.rels"];'
         'if(!a||!b)return Promise.resolve();return Promise.all([a.async("text"),b.async("text")]).then((function(x){'
         'var rel={},m,re=/<Relationship\\b[^>]*>/g;while(m=re.exec(x[1])){var id=/\\bId="([^"]+)"/.exec(m[0]),tg=/\\bTarget="([^"]+)"/.exec(m[0]);'
         'if(id&&tg)rel[id[1]]=tg[1].charAt(0)==="/"?tg[1].substr(1):"ppt/"+tg[1].replace(/^\\.\\//,"")}'
         'var ord={},i=0,r2=/<p:sldId\\b[^>]*\\br:id="([^"]+)"/g;while(m=r2.exec(x[0]))rel[m[1]]&&!(rel[m[1]] in ord)&&(ord[rel[m[1]]]=i++);'
         'i&&(s.slides=s.slides.filter((function(z){return z.name in ord})).sort((function(p,q){return ord[p.name]-ord[q.name]})))'
         '})).catch((function(e){console.warn("pptx order",e)}))},')
rep('t.prototype._loadThumbnail=function(){', ORDER + 't.prototype._loadThumbnail=function(){')

# 5. 形状写了 <a:ln><a:noFill/></a:ln>（无边框）时不再套用主题 lnRef 的边框；原库 wr() 对 noFill 返回 undefined，Object.assign 什么也不做
rep('var i=Yn.get(this.source,["p:spPr","a:ln"]);i&&Object.assign(this.border,wr(i,this.theme,this))',
    'var i=Yn.get(this.source,["p:spPr","a:ln"]);i&&(Yn.get(i,"a:noFill")?this.border={}/* Peek 补丁：无边框 */:Object.assign(this.border,wr(i,this.theme,this)))')

# 6. 母版/版式里所有非占位符形状都要画（PowerPoint/WPS 的行为）；原库只画带 userDrawn="1" 的，装饰条、线条、色块大多丢失
rep('var a=Yn.get(o,"p:ph");a&&a.attrs&&(this.idx=a.attrs.idx,this.type=a.attrs.type)',
    'var a=Yn.get(o,"p:ph");a&&(this.isPh=!0),a&&a.attrs&&(this.idx=a.attrs.idx,this.type=a.attrs.type)')
rep('var r=c([],t.nodes,!0).filter((function(t){return t.userDrawn}));',
    'var r=c([],t.nodes,!0).filter((function(t){return t.userDrawn||!t.isPh}))/* Peek 补丁 */;', count=2)

# 7. 幻灯片/版式 showMasterSp="0" 时不画母版（和版式）上的图形；母版或版式缺失时跳过而不是报错
rep('this._renderSlideMaster(e.slideMaster,n),this._renderSlideLayout(e.slideLayout,n),',
    '(function(x){var r=x&&x.source&&(x.source["p:sld"]||x.source["p:sldLayout"]);return!(r&&r.attrs&&"0"===r.attrs.showMasterSp)})/* Peek 补丁 */.call(null,e)'
    '&&(e.slideMaster&&(function(x){var r=x&&x.source&&x.source["p:sldLayout"];return!(r&&r.attrs&&"0"===r.attrs.showMasterSp)})(e.slideLayout)&&this._renderSlideMaster(e.slideMaster,n),e.slideLayout&&this._renderSlideLayout(e.slideLayout,n)),')

# 8. 段落对齐：补上两端对齐 just（中文文本框最常见），没写对齐时按 PowerPoint 默认左对齐；原库都当居中
rep('v.style.textAlign=d.align&&{ctr:"center",l:"left",r:"right",dist:"justify"}[d.align]||"center"',
    'v.style.textAlign=d.align&&{ctr:"center",l:"left",r:"right",dist:"justify",just:"justify",justLow:"justify",thaiDist:"justify"}[d.align]||"left"/* Peek 补丁 */')

# 9. 预设颜色 prstClr 上的亮度/透明度修饰（lumMod/lumOff 等）也要生效，原库只认 srgb/scheme/sys
rep('var a=t["a:srgbClr"]||t["a:schemeClr"]||t["a:sysClr"],',
    'var a=t["a:srgbClr"]||t["a:schemeClr"]||t["a:sysClr"]||t["a:prstClr"]/* Peek 补丁 */,')

# 10. 正文占位符继承母版 bodyStyle（项目符号、字体、字号）；原库只给标题套 titleStyle，正文占位符的 bodyStyle 解析了却从没用上
for k in ('props', 'defRPr'):
    rep('["subTitle","ctrTitle","title"].includes(i)&&Object.assign(n,Yn.get(r.slideLayout.slideMaster,["textStyles","titleStyle","lvl".concat(t?+t+1:1),"%s"])||{})' % k,
        '(["ctrTitle","title"].includes(i)?Object.assign(n,Yn.get(r.slideLayout.slideMaster,["textStyles","titleStyle","lvl".concat(t?+t+1:1),"%s"])||{})'
        ':["dt","ftr","sldNum","hdr"].includes(i)||Object.assign(n,Yn.get(r.slideLayout.slideMaster,["textStyles","bodyStyle","lvl".concat(t?+t+1:1),"%s"])||{}))/* Peek 补丁 */' % (k, k))

# 11. 形状自己 <a:lstStyle> 里的段落属性（项目符号、缩进、对齐）也要继承；原库只对字符属性 defRPr 用了它
rep('}return n},t.prototype._getInheritRProps',
    '}return Yn.get(this.lstStyle,["lvl".concat(t?+t+1:1),"props"])&&Object.assign(n,Yn.get(this.lstStyle,["lvl".concat(t?+t+1:1),"props"]))/* Peek 补丁 */,n},t.prototype._getInheritRProps')

# 12. 项目符号的大小（buSzPct）和颜色（buClr）；原库符号永远和第一个字一样大、一样色
rep('Yn.get(t,["a:buChar","attrs","char"])&&(e.buChar=t["a:buChar"].attrs.char),',
    'Yn.get(t,["a:buChar","attrs","char"])&&(e.buChar=t["a:buChar"].attrs.char),'
    'Yn.get(t,["a:buSzPct","attrs","val"])&&(e.buSzPct=parseInt(t["a:buSzPct"].attrs.val)/1e5),'
    'function(s){try{t["a:buClr"]&&s.node&&s.node.theme&&(e.buClr=ir(t["a:buClr"],s.node.theme,s.node))}catch(x){}}(this)/* Peek 补丁 */,', count=2)
rep('n.style.color=r.style.color,n.style.fontSize=r.style.fontSize,n.textContent={n:"■"',
    'n.style.color=e.buClr||r.style.color,n.style.fontSize=parseFloat(r.style.fontSize)*(e.buSzPct||1)+"px",n.style.verticalAlign="middle"/* Peek 补丁 */,n.textContent={n:"■"')

open(P, 'w', encoding='utf8', newline='').write(s)
print('patched')
