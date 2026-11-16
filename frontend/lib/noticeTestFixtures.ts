import type { Notice, NoticeSource } from './noticeGateway';
export const source:NoticeSource={pluginId:'observer',displayName:'Observer',enabled:true,connected:true,message:''};
export const notice=(id:string,revision=1,finished=false):Notice=>({noticeId:id,pluginId:'observer',sourceId:'chat',revision,finished,status:{value:'provider-defined',label:'由接入方定义'},content:{title:id},actions:[],createdAt:1,updatedAt:revision});
